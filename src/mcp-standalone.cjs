'use strict';

/**
 * 橙子日程 MCP 服务器 —— 独立模式（standalone）
 *
 * 不依赖 Electron 主进程，不打开任何窗口，不使用系统通知。
 * 纯 Node.js 进程，通过 stdio 与 MCP 客户端（如 Claude Desktop）通信，
 * 直接读写本地数据文件 plans.json。
 *
 * 启动方式（MCP 客户端必须用 node 直接启动，不要用 npm run，
 * 因为 npm 会把脚本信息写到 stdout，污染 JSON-RPC 流）：
 *   node src/mcp-standalone.cjs
 *
 * 运行边界：
 *   本进程每次操作都直接读写 plans.json。若同时运行 Electron 主程序，
 *   主程序基于内存状态整体写回文件，可能覆盖本进程的改动。
 *   建议主程序退出后再使用独立模式，或只让其中一方写入。
 *
 * 数据目录优先级：
 *   1. WEEKLIGHT_DATA_DIR 环境变量
 *   2. Windows: %APPDATA%\weeklight
 *   3. 其他平台: ~/.config/weeklight
 */

const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');

let Server, StdioServerTransport;
let ListResourcesRequestSchema, ReadResourceRequestSchema;
let ListToolsRequestSchema, CallToolRequestSchema;
let ListPromptsRequestSchema, GetPromptRequestSchema;

try {
  const serverEntry = require.resolve('@modelcontextprotocol/sdk/server');
  ({ Server } = require(serverEntry));
  // exports 映射中的通配子路径不带 .js 后缀，直接按文件位置加载
  const serverDir = path.dirname(serverEntry);
  ({ StdioServerTransport } = require(path.join(serverDir, 'stdio.js')));
  ({
    ListResourcesRequestSchema,
    ReadResourceRequestSchema,
    ListToolsRequestSchema,
    CallToolRequestSchema,
    ListPromptsRequestSchema,
    GetPromptRequestSchema
  } = require(path.join(serverDir, '..', 'types.js')));
} catch (err) {
  // 兼容旧版 SDK 的顶层导出
  const sdk = require('@modelcontextprotocol/sdk');
  Server = sdk.Server;
  StdioServerTransport = sdk.StdioServerTransport;
  ({
    ListResourcesRequestSchema,
    ReadResourceRequestSchema,
    ListToolsRequestSchema,
    CallToolRequestSchema,
    ListPromptsRequestSchema,
    GetPromptRequestSchema
  } = require('@modelcontextprotocol/sdk/types'));
}

const {
  cleanTask,
  dueTasks,
  makeSeries,
  validateState
} = require('./core.cjs');

const {
  previewTaskAction,
  applyTaskAction,
  validateSeriesTasks
} = require('./task-series.cjs');

const { normalizePrefs } = require('./widget-model.cjs');

const CATEGORIES = ['work', 'study', 'life'];

// ---------- 数据目录与文件 ----------

function resolveDataDir() {
  if (process.env.WEEKLIGHT_DATA_DIR) {
    return path.resolve(process.env.WEEKLIGHT_DATA_DIR);
  }
  if (os.platform() === 'win32') {
    return path.join(os.homedir(), 'AppData', 'Roaming', 'weeklight');
  }
  return path.join(os.homedir(), '.config', 'weeklight');
}

const DATA_DIR = resolveDataDir();
const DATA_FILE = path.join(DATA_DIR, 'plans.json');

function ensureDir() {
  fs.mkdirSync(DATA_DIR, { recursive: true });
}

// validateState 会刷新任务的 updatedAt；缓存文件内容未变时的状态，
// 保证 preview_task_action 返回的确认令牌在后续操作中保持一致。
let cachedRaw = null;
let cachedState = null;

function readState() {
  if (!fs.existsSync(DATA_FILE)) {
    cachedRaw = null;
    cachedState = { version: 1, tasks: [], settings: {}, history: [] };
    return structuredClone(cachedState);
  }
  const raw = fs.readFileSync(DATA_FILE, 'utf8');
  if (raw !== cachedRaw) {
    cachedRaw = raw;
    cachedState = validateState(JSON.parse(raw.replace(/^\uFEFF/, '')));
  }
  return structuredClone(cachedState);
}

function writeState(state) {
  ensureDir();
  const tmp = DATA_FILE + '.tmp';
  const serialized = JSON.stringify(state, null, 2);
  fs.writeFileSync(tmp, serialized, 'utf8');
  if (fs.existsSync(DATA_FILE)) {
    fs.copyFileSync(DATA_FILE, DATA_FILE + '.bak');
  }
  fs.renameSync(tmp, DATA_FILE);
  cachedRaw = serialized;
  cachedState = structuredClone(state);
}

/**
 * 事务性修改：读取 -> 克隆 -> 修改 -> 持久化 -> 返回新状态。
 * 写入采用临时文件 + .bak 副本 + rename，与主程序保存方式一致。
 */
function change(fn) {
  const next = structuredClone(readState());
  fn(next);
  writeState(next);
  return next;
}

// ---------- 辅助 ----------

function taskSummary(task) {
  return {
    id: task.id,
    title: task.title,
    date: task.date,
    time: task.time,
    duration: task.duration,
    reminder: task.reminder,
    category: task.category,
    priority: task.priority,
    notes: task.notes,
    done: !!task.done,
    createdAt: task.createdAt,
    updatedAt: task.updatedAt,
    notifiedAt: task.notifiedAt ?? null,
    snoozeUntil: task.snoozeUntil ?? null,
    seriesId: task.seriesId ?? null,
    seriesIndex: task.seriesIndex ?? null
  };
}

function filterTasks(tasks, { from, until, done, category }) {
  return tasks.filter(t => {
    if (from && t.date < from) return false;
    if (until && t.date > until) return false;
    if (done !== undefined && !!t.done !== done) return false;
    if (category && t.category !== category) return false;
    return true;
  });
}

function isValidId(id) {
  return typeof id === 'string' && /^[a-zA-Z0-9_-]{1,80}$/.test(id);
}

// ---------- MCP 服务器 ----------

module.exports = async function createStandaloneMcpServer() {
  const server = new Server(
    { name: 'orange-schedule', version: '1.2.1' },
    { capabilities: { resources: {}, tools: {}, prompts: {} } }
  );

  // ================= Resources =================

  server.setRequestHandler(ListResourcesRequestSchema, async () => ({
    resources: [
      {
        uri: 'weeklight://plans',
        name: '所有计划',
        description: '橙子日程全部计划列表（含重复分组与完成状态）',
        mimeType: 'application/json'
      },
      {
        uri: 'weeklight://settings',
        name: '应用设置',
        description: '界面语言、声音、弹窗、自动更新、便签偏好等',
        mimeType: 'application/json'
      },
      {
        uri: 'weeklight://reminders',
        name: '提醒文案库',
        description: '120 条古典名言警句与诗词及出处',
        mimeType: 'application/json'
      },
      {
        uri: 'weeklight://data-dir',
        name: '数据目录',
        description: '当前 plans.json 所在目录路径',
        mimeType: 'application/json'
      }
    ]
  }));

  server.setRequestHandler(ReadResourceRequestSchema, async ({ params: { uri } }) => {
    const state = readState();
    let contents;

    if (uri === 'weeklight://plans') {
      contents = JSON.stringify(state.tasks.map(taskSummary), null, 2);
    } else if (uri === 'weeklight://settings') {
      contents = JSON.stringify(
        {
          language: state.settings.language ?? 'zh-CN',
          sound: state.settings.sound !== false,
          popup: state.settings.popup !== false,
          checkUpdates: state.settings.checkUpdates !== false,
          widget: normalizePrefs(state.settings.widget)
        },
        null,
        2
      );
    } else if (uri === 'weeklight://reminders') {
      const { messages } = require('./reminder-copy.cjs');
      contents = JSON.stringify(
        messages.map(m => ({ id: m.id, text: m.text, source: m.source, url: m.url })),
        null,
        2
      );
    } else if (uri === 'weeklight://data-dir') {
      contents = JSON.stringify(
        { dataDir: DATA_DIR, plansFile: DATA_FILE },
        null,
        2
      );
    } else if (/^weeklight:\/\/plan\/.+/.test(uri)) {
      const id = uri.slice('weeklight://plan/'.length);
      if (!isValidId(id)) throw new Error(`无效的计划 ID：${id}`);
      const task = state.tasks.find(t => t.id === id);
      if (!task) throw new Error(`计划不存在：${id}`);
      contents = JSON.stringify(taskSummary(task), null, 2);
    } else {
      throw new Error(`未知资源：${uri}`);
    }

    return {
      contents: [{ uri, mimeType: 'application/json', text: contents }]
    };
  });

  // ================= Tools =================

  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: [
      {
        name: 'weeklight.list_tasks',
        description: '列出计划，支持按日期范围、完成状态、分类过滤',
        inputSchema: {
          type: 'object',
          properties: {
            from: { type: 'string', description: '开始日期 YYYY-MM-DD' },
            until: { type: 'string', description: '结束日期 YYYY-MM-DD' },
            done: { type: 'boolean', description: 'true=仅已完成，false=仅待办' },
            category: { type: 'string', enum: CATEGORIES, description: '分类：work/study/life' }
          },
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.get_task',
        description: '按 ID 获取单个计划详情',
        inputSchema: {
          type: 'object',
          properties: { id: { type: 'string', description: '计划 ID' } },
          required: ['id'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.create_task',
        description: '新建计划，支持单次或重复系列（每天/工作日/每周 4/8/12 周）',
        inputSchema: {
          type: 'object',
          properties: {
            title: { type: 'string', description: '计划名称（1–160 字）' },
            date: { type: 'string', description: '日期 YYYY-MM-DD' },
            time: { type: 'string', description: '时间 HH:mm' },
            duration: { type: 'integer', description: '时长 15–1440 分钟', default: 60 },
            reminder: {
              type: 'integer',
              description: '提前提醒分钟：-1=不提醒,0=准点,5/10/15/30/60/1440',
              default: 10
            },
            category: { type: 'string', enum: CATEGORIES, description: '分类', default: 'life' },
            priority: { type: 'string', enum: ['high', 'normal'], description: '优先级', default: 'normal' },
            notes: { type: 'string', description: '备注，最多 5000 字', default: '' },
            repeat: {
              description: '重复设置：1=单次，4/8/12=每周，或 {frequency,until} 每天/工作日',
              oneOf: [
                { type: 'integer', enum: [1, 4, 8, 12] },
                {
                  type: 'object',
                  properties: {
                    frequency: { type: 'string', enum: ['daily', 'weekdays'] },
                    until: { type: 'string', description: '重复结束日期 YYYY-MM-DD' }
                  },
                  required: ['frequency', 'until'],
                  additionalProperties: false
                }
              ],
              default: 1
            }
          },
          required: ['title', 'date', 'time'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.update_task',
        description: '修改现有计划（仅传要改动的字段）',
        inputSchema: {
          type: 'object',
          properties: {
            id: { type: 'string', description: '计划 ID' },
            title: { type: 'string', description: '计划名称（1–160 字）' },
            date: { type: 'string', description: '日期 YYYY-MM-DD' },
            time: { type: 'string', description: '时间 HH:mm' },
            duration: { type: 'integer', description: '时长 15–1440 分钟' },
            reminder: { type: 'integer', description: '提前提醒分钟' },
            category: { type: 'string', enum: CATEGORIES, description: '分类' },
            priority: { type: 'string', enum: ['high', 'normal'], description: '优先级' },
            notes: { type: 'string', description: '备注，最多 5000 字' },
            done: { type: 'boolean', description: '完成状态' }
          },
          required: ['id'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.delete_task',
        description: '删除计划，支持 single（仅此项）或 following（此项及后续）',
        inputSchema: {
          type: 'object',
          properties: {
            id: { type: 'string', description: '计划 ID' },
            scope: { type: 'string', enum: ['single', 'following'], default: 'single' },
            token: {
              type: 'string',
              description: '操作确认令牌，先调用 weeklight.preview_task_action 获取'
            }
          },
          required: ['id'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.preview_task_action',
        description: '预览对计划的操作影响范围，并返回确认令牌',
        inputSchema: {
          type: 'object',
          properties: {
            id: { type: 'string', description: '计划 ID' },
            action: { type: 'string', enum: ['complete', 'reopen', 'delete'] }
          },
          required: ['id', 'action'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.complete_task',
        description: '标记计划完成或重新打开为待办，支持 single/following',
        inputSchema: {
          type: 'object',
          properties: {
            id: { type: 'string', description: '计划 ID' },
            done: { type: 'boolean', description: 'true=完成，false=重新打开' },
            scope: { type: 'string', enum: ['single', 'following'], default: 'single' },
            token: {
              type: 'string',
              description: '操作确认令牌，先调用 weeklight.preview_task_action 获取'
            }
          },
          required: ['id', 'done'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.move_task',
        description: '将计划改期或改时',
        inputSchema: {
          type: 'object',
          properties: {
            id: { type: 'string', description: '计划 ID' },
            date: { type: 'string', description: '新日期 YYYY-MM-DD' },
            time: { type: 'string', description: '新时间 HH:mm（不传则保持原时间）' }
          },
          required: ['id', 'date'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.reminder_action',
        description: '对计划提醒执行操作：稍后 10 分钟 / 完成 / 忽略',
        inputSchema: {
          type: 'object',
          properties: {
            id: { type: 'string', description: '计划 ID' },
            action: { type: 'string', enum: ['snooze', 'done', 'dismiss'] }
          },
          required: ['id', 'action'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.due_tasks',
        description: '获取当前到期或即将提醒的计划',
        inputSchema: {
          type: 'object',
          properties: {
            now: { type: 'integer', description: '时间戳（毫秒），默认当前时间' }
          },
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.export_backup',
        description: '导出 JSON 备份到指定文件路径',
        inputSchema: {
          type: 'object',
          properties: {
            filePath: { type: 'string', description: '备份文件保存路径（如 C:\\backup\\plans.json）' }
          },
          required: ['filePath'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.import_backup',
        description: '从 JSON 备份文件导入并合并（相同 ID 使用备份内容，其余保留）',
        inputSchema: {
          type: 'object',
          properties: {
            filePath: { type: 'string', description: '备份文件路径' }
          },
          required: ['filePath'],
          additionalProperties: false
        }
      },
      {
        name: 'weeklight.get_settings',
        description: '读取当前应用设置'
      },
      {
        name: 'weeklight.update_settings',
        description: '修改应用设置（独立模式下不支持开机自动启动）',
        inputSchema: {
          type: 'object',
          properties: {
            language: { type: 'string', enum: ['zh-CN', 'en'] },
            sound: { type: 'boolean' },
            popup: { type: 'boolean' },
            checkUpdates: { type: 'boolean' }
          },
          additionalProperties: false
        }
      }
    ]
  }));

  server.setRequestHandler(CallToolRequestSchema, async ({ params: { name, arguments: args = {} } }) => {
    try {
      switch (name) {
        case 'weeklight.list_tasks': {
          const state = readState();
          const list = filterTasks(state.tasks, {
            from: args.from,
            until: args.until,
            done: args.done,
            category: args.category
          });
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({ count: list.length, tasks: list.map(taskSummary) }, null, 2)
              }
            ]
          };
        }

        case 'weeklight.get_task': {
          const state = readState();
          const task = state.tasks.find(t => t.id === args.id);
          if (!task) throw new Error(`计划不存在：${args.id}`);
          return { content: [{ type: 'text', text: JSON.stringify(taskSummary(task), null, 2) }] };
        }

        case 'weeklight.create_task': {
          const draft = {
            title: args.title,
            date: args.date,
            time: args.time,
            duration: args.duration ?? 60,
            reminder: args.reminder ?? 10,
            category: args.category ?? 'life',
            priority: args.priority ?? 'normal',
            notes: args.notes ?? ''
          };
          const task = cleanTask(draft);
          const created = makeSeries(task, args.repeat ?? 1);
          const current = readState();
          if (current.tasks.length + created.length > 10000) {
            throw new Error('计划总数不能超过 10000 项');
          }
          const result = change(s => {
            s.tasks.push(...created);
          });
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(
                  { created: created.map(taskSummary), total: result.tasks.length },
                  null,
                  2
                )
              }
            ]
          };
        }

        case 'weeklight.update_task': {
          const result = change(s => {
            const old = s.tasks.find(t => t.id === args.id);
            if (!old) throw new Error(`计划不存在：${args.id}`);
            const next = cleanTask({ ...old, ...args }, old);
            s.tasks = s.tasks.map(x => (x.id === args.id ? next : x));
          });
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(
                  { ok: true, task: taskSummary(result.tasks.find(t => t.id === args.id)) },
                  null,
                  2
                )
              }
            ]
          };
        }

        case 'weeklight.preview_task_action': {
          const state = readState();
          const preview = previewTaskAction(state.tasks, args.id, args.action);
          return {
            content: [{ type: 'text', text: JSON.stringify(preview, null, 2) }]
          };
        }

        case 'weeklight.delete_task': {
          const state = readState();
          const action = 'delete';
          const preview = previewTaskAction(state.tasks, args.id, action);

          if (args.scope === 'following') {
            if (typeof args.token !== 'string' || args.token !== preview.token) {
              throw new Error('事项已发生变化，请重新确认操作范围');
            }
          } else if (args.scope === 'single') {
            if (typeof args.token !== 'string' || args.token !== preview.singleToken) {
              throw new Error('事项已发生变化，请重新确认删除');
            }
          }

          const ids = new Set(
            args.scope === 'following' ? preview.items.map(t => t.id) : [args.id]
          );
          change(s => {
            s.tasks = s.tasks.filter(t => !ids.has(t.id));
            s.history = s.history.filter(h => !ids.has(h.taskId));
          });
          return {
            content: [{ type: 'text', text: JSON.stringify({ deleted: [...ids] }, null, 2) }]
          };
        }

        case 'weeklight.complete_task': {
          const state = readState();
          const action = args.done ? 'complete' : 'reopen';
          const preview = previewTaskAction(state.tasks, args.id, action);

          if (args.scope === 'following') {
            if (typeof args.token !== 'string' || args.token !== preview.token) {
              throw new Error('事项已发生变化，请重新确认操作范围');
            }
          }

          const ids = new Set(
            args.scope === 'following' ? preview.items.map(t => t.id) : [args.id]
          );
          change(s => {
            for (const task of s.tasks) {
              if (ids.has(task.id)) {
                task.done = args.done;
                task.snoozeUntil = null;
                task.updatedAt = Date.now();
              }
            }
            if (args.done) {
              s.history.forEach(h => {
                if (ids.has(h.taskId)) h.dismissed = true;
              });
            }
          });
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({ ok: true, done: args.done, ids: [...ids] }, null, 2)
              }
            ]
          };
        }

        case 'weeklight.move_task': {
          const result = change(s => {
            const old = s.tasks.find(x => x.id === args.id);
            if (!old) throw new Error(`计划不存在：${args.id}`);
            const next = cleanTask(
              { ...old, date: args.date, time: args.time || old.time },
              old
            );
            s.tasks = s.tasks.map(x => (x.id === args.id ? next : x));
            s.history.forEach(h => {
              if (h.taskId === args.id) h.dismissed = true;
            });
          });
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(
                  { ok: true, task: taskSummary(result.tasks.find(t => t.id === args.id)) },
                  null,
                  2
                )
              }
            ]
          };
        }

        case 'weeklight.reminder_action': {
          const result = change(s => {
            const t = s.tasks.find(x => x.id === args.id);
            if (!t) return;
            if (args.action === 'snooze') {
              t.snoozeUntil = Date.now() + 600000;
            } else if (args.action === 'done') {
              t.done = true;
              t.snoozeUntil = null;
            } else if (args.action !== 'dismiss') {
              throw new Error('操作无效');
            }
            s.history.forEach(h => {
              if (h.taskId === args.id) h.dismissed = true;
            });
          });
          return { content: [{ type: 'text', text: JSON.stringify({ ok: true, action: args.action }, null, 2) }] };
        }

        case 'weeklight.due_tasks': {
          const state = readState();
          const list = dueTasks(state.tasks, args.now);
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({ count: list.length, tasks: list.map(taskSummary) }, null, 2)
              }
            ]
          };
        }

        case 'weeklight.export_backup': {
          const state = readState();
          fs.mkdirSync(path.dirname(args.filePath), { recursive: true });
          fs.writeFileSync(args.filePath, JSON.stringify(state, null, 2), 'utf8');
          return {
            content: [{ type: 'text', text: JSON.stringify({ saved: args.filePath }, null, 2) }]
          };
        }

        case 'weeklight.import_backup': {
          const raw = JSON.parse(fs.readFileSync(args.filePath, 'utf8').replace(/^\uFEFF/, ''));
          const incoming = validateState(raw);
          change(s => {
            const map = new Map(s.tasks.map(task => [task.id, task]));
            incoming.tasks.forEach(task => {
              const old = map.get(task.id);
              if (old?.seriesId && task.seriesId === undefined) {
                task = { ...task, seriesId: old.seriesId, seriesIndex: old.seriesIndex };
              }
              map.set(task.id, task);
            });
            validateSeriesTasks([...map.values()]);
            if (map.size > 10000) throw new Error('合并后超过 10000 项上限');
            s.tasks = [...map.values()];
          });
          const after = readState();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({ imported: incoming.tasks.length, total: after.tasks.length }, null, 2)
              }
            ]
          };
        }

        case 'weeklight.get_settings': {
          const state = readState();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(
                  {
                    language: state.settings.language ?? 'zh-CN',
                    sound: state.settings.sound !== false,
                    popup: state.settings.popup !== false,
                    checkUpdates: state.settings.checkUpdates !== false,
                    widget: normalizePrefs(state.settings.widget)
                  },
                  null,
                  2
                )
              }
            ]
          };
        }

        case 'weeklight.update_settings': {
          const result = change(s => {
            if (typeof args.language === 'string') {
              if (!['zh-CN', 'en'].includes(args.language)) {
                throw new Error('不支持的界面语言');
              }
              s.settings.language = args.language;
            }
            if (typeof args.sound === 'boolean') {
              s.settings.sound = args.sound;
            }
            if (typeof args.popup === 'boolean') {
              s.settings.popup = args.popup;
            }
            if (typeof args.checkUpdates === 'boolean') {
              s.settings.checkUpdates = args.checkUpdates;
            }
          });
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({ ok: true, settings: result.settings }, null, 2)
              }
            ]
          };
        }

        default:
          throw new Error(`未知工具：${name}`);
      }
    } catch (err) {
      return {
        isError: true,
        content: [{ type: 'text', text: String(err.message ?? err) }]
      };
    }
  });

  // ================= Prompts =================

  server.setRequestHandler(ListPromptsRequestSchema, async () => ({
    prompts: [
      {
        name: 'weeklight.plan-week',
        description: '基于现有计划规划下一周，标出冲突与空闲',
        arguments: [
          { name: 'from', description: '开始日期 YYYY-MM-DD' },
          { name: 'until', description: '结束日期 YYYY-MM-DD' }
        ]
      },
      {
        name: 'weeklight.review-tasks',
        description: '回顾待办事项，按日期与分类汇总',
        arguments: [
          { name: 'category', description: '分类 work/study/life（可选）' }
        ]
      },
      {
        name: 'weeklight.reminder-copy',
        description: '说明 120 条提醒文案的来源与轮换规则'
      }
    ]
  }));

  server.setRequestHandler(GetPromptRequestSchema, async ({ params: { name, arguments: args = {} } }) => {
    const state = readState();

    if (name === 'weeklight.plan-week') {
      const from = args.from || new Date().toISOString().slice(0, 10);
      const until = args.until || from;
      const list = filterTasks(state.tasks, { from, until, done: false });
      return {
        messages: [
          {
            role: 'user',
            content: {
              type: 'text',
              text:
                `以下是 ${from} 至 ${until} 的待办计划（共 ${list.length} 项）：\n` +
                list.map(t => `- ${t.date} ${t.time} [${t.category}] ${t.title}`).join('\n') +
                '\n\n请帮助评估是否有时间冲突，并建议空闲时段可安排的新事项。'
            }
          }
        ]
      };
    }

    if (name === 'weeklight.review-tasks') {
      const list = filterTasks(state.tasks, { done: false, category: args.category });
      const byCategory = {};
      for (const t of list) {
        (byCategory[t.category] = byCategory[t.category] || []).push(t);
      }
      return {
        messages: [
          {
            role: 'user',
            content: {
              type: 'text',
              text:
                '待办事项回顾（共 ' +
                list.length +
                ' 项）：\n' +
                Object.entries(byCategory)
                  .map(
                    ([cat, items]) =>
                      `【${cat}】（${items.length} 项）：\n` +
                      items.map(t => `- ${t.date} ${t.time} ${t.title}`).join('\n')
                  )
                  .join('\n\n') +
                '\n\n请按紧急程度排序，并建议哪些可以稍后处理。'
            }
          }
        ]
      };
    }

    if (name === 'weeklight.reminder-copy') {
      const { messages } = require('./reminder-copy.cjs');
      return {
        messages: [
          {
            role: 'user',
            content: {
              type: 'text',
              text:
                `橙子日程内置 ${messages.length} 条提醒文案，全部选自古典名言警句与诗词（如《论语》《荀子》、唐宋诗词），每轮随机展示且不重复，重启后继续轮换。文案带作品出处，可用于提醒弹窗与系统通知。`
            }
          }
        ]
      };
    }

    throw new Error(`未知提示：${name}`);
  });

  const transport = new StdioServerTransport();
  await server.connect(transport);

  return { server, transport };
};

// 直接运行时启动 MCP 服务器
if (require.main === module) {
  module.exports().catch(err => {
    console.error(String(err.stack ?? err));
    process.exit(1);
  });
}
