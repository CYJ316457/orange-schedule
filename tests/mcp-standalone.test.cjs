const test = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const readline = require('node:readline');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const MCP_SCRIPT = path.join(__dirname, '..', 'src', 'mcp-standalone.cjs');

function createTempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'mcp-standalone-test-'));
}

function spawnMcp(dataDir) {
  const proc = spawn(process.execPath, [MCP_SCRIPT], {
    env: { ...process.env, WEEKLIGHT_DATA_DIR: dataDir },
    stdio: ['pipe', 'pipe', 'pipe']
  });

  const rl = readline.createInterface({ input: proc.stdout });
  const buffer = [];
  rl.on('line', line => buffer.push(line));

  function request(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = Math.floor(Math.random() * 1e9);
      const msg = JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n';
      proc.stdin.write(msg, err => {
        if (err) return reject(err);
      });

      const timeout = setTimeout(() => reject(new Error(`Timeout waiting for ${method}`)), 10000);

      const poll = () => {
        const idx = buffer.findIndex(line => {
          try {
            const parsed = JSON.parse(line);
            return parsed && parsed.id === id;
          } catch {
            return false;
          }
        });
        if (idx >= 0) {
          clearTimeout(timeout);
          resolve(JSON.parse(buffer.splice(idx, 1)[0]));
        } else {
          setTimeout(poll, 20);
        }
      };
      poll();
    });
  }

  return { proc, request, close: () => proc.kill() };
}

function init(mcp) {
  return mcp.request('initialize', {
    protocolVersion: '2024-11-05',
    capabilities: {},
    clientInfo: { name: 'test', version: '1.0.0' }
  });
}

test('mcp standalone: initialize and list tools', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);
      const res = await mcp.request('tools/list', {});
      assert.ok(!res.error, JSON.stringify(res.error));
      const tools = res.result.tools;
      assert.ok(Array.isArray(tools));
      const names = tools.map(t => t.name);
      assert.ok(names.includes('weeklight.list_tasks'));
      assert.ok(names.includes('weeklight.create_task'));
      assert.ok(names.includes('weeklight.get_task'));
      assert.ok(names.includes('weeklight.update_task'));
      assert.ok(names.includes('weeklight.delete_task'));
      assert.ok(names.includes('weeklight.complete_task'));
      assert.ok(names.includes('weeklight.move_task'));
      assert.ok(names.includes('weeklight.due_tasks'));
      assert.ok(names.includes('weeklight.export_backup'));
      assert.ok(names.includes('weeklight.import_backup'));
      assert.ok(names.includes('weeklight.get_settings'));
      assert.ok(names.includes('weeklight.update_settings'));
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: create task and list tasks', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const create = await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: {
          title: '测试计划',
          date: '2026-11-10',
          time: '10:00',
          duration: 60,
          reminder: 10,
          category: 'work'
        }
      });
      assert.ok(!create.result.isError, JSON.stringify(create.result));
      const created = JSON.parse(create.result.content[0].text);
      assert.equal(created.created.length, 1);
      assert.equal(created.created[0].title, '测试计划');
      assert.equal(created.created[0].category, 'work');
      assert.equal(created.created[0].reminder, 10);
      const taskId = created.created[0].id;

      const list = await mcp.request('tools/call', {
        name: 'weeklight.list_tasks',
        arguments: {}
      });
      assert.ok(!list.result.isError);
      const listData = JSON.parse(list.result.content[0].text);
      assert.equal(listData.count, 1);
      assert.equal(listData.tasks[0].id, taskId);

      const get = await mcp.request('tools/call', {
        name: 'weeklight.get_task',
        arguments: { id: taskId }
      });
      assert.ok(!get.result.isError);
      assert.equal(JSON.parse(get.result.content[0].text).title, '测试计划');
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: update and complete task', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const create = await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: { title: '原标题', date: '2026-11-10', time: '10:00' }
      });
      assert.ok(!create.result.isError);
      const taskId = JSON.parse(create.result.content[0].text).created[0].id;

      const updated = await mcp.request('tools/call', {
        name: 'weeklight.update_task',
        arguments: { id: taskId, title: '新标题', duration: 90 }
      });
      assert.ok(!updated.result.isError);
      const updatedTask = JSON.parse(updated.result.content[0].text).task;
      assert.equal(updatedTask.title, '新标题');
      assert.equal(updatedTask.duration, 90);

      const completed = await mcp.request('tools/call', {
        name: 'weeklight.complete_task',
        arguments: { id: taskId, done: true }
      });
      assert.ok(!completed.result.isError);
      const doneData = JSON.parse(completed.result.content[0].text);
      assert.equal(doneData.done, true);

      const listDone = await mcp.request('tools/call', {
        name: 'weeklight.list_tasks',
        arguments: { done: true }
      });
      assert.ok(!listDone.result.isError);
      assert.equal(JSON.parse(listDone.result.content[0].text).count, 1);

      const listPending = await mcp.request('tools/call', {
        name: 'weeklight.list_tasks',
        arguments: { done: false }
      });
      assert.ok(!listPending.result.isError);
      assert.equal(JSON.parse(listPending.result.content[0].text).count, 0);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: create repeating daily series', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const create = await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: {
          title: '每日复盘',
          date: '2026-11-10',
          time: '09:00',
          repeat: { frequency: 'daily', until: '2026-11-12' }
        }
      });
      assert.ok(!create.result.isError, JSON.stringify(create.result));
      const created = JSON.parse(create.result.content[0].text);
      assert.equal(created.created.length, 3);
      assert.deepEqual(created.created.map(t => t.date), [
        '2026-11-10',
        '2026-11-11',
        '2026-11-12'
      ]);
      assert.ok(new Set(created.created.map(t => t.id)).size === 3);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: delete task', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const create = await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: { title: '待删除', date: '2026-11-10', time: '10:00' }
      });
      assert.ok(!create.result.isError);
      const taskId = JSON.parse(create.result.content[0].text).created[0].id;

      // 先预览获取 token
      const preview = await mcp.request('tools/call', {
        name: 'weeklight.preview_task_action',
        arguments: { id: taskId, action: 'delete' }
      });
      assert.ok(!preview.result.isError);
      const previewData = JSON.parse(preview.result.content[0].text);
      assert.equal(previewData.source, 'single');

      const deleted = await mcp.request('tools/call', {
        name: 'weeklight.delete_task',
        arguments: { id: taskId, scope: 'single', token: previewData.singleToken }
      });
      assert.ok(!deleted.result.isError, JSON.stringify(deleted.result));

      const list = await mcp.request('tools/call', {
        name: 'weeklight.list_tasks',
        arguments: {}
      });
      assert.equal(JSON.parse(list.result.content[0].text).count, 0);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: export and import backup', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: { title: '备份测试', date: '2026-11-10', time: '10:00' }
      });

      const backupFile = path.join(dir, 'backup.json');
      const exported = await mcp.request('tools/call', {
        name: 'weeklight.export_backup',
        arguments: { filePath: backupFile }
      });
      assert.ok(!exported.result.isError);
      assert.ok(fs.existsSync(backupFile));

      // 清空后导入
      const listBefore = await mcp.request('tools/call', {
        name: 'weeklight.list_tasks',
        arguments: {}
      });
      assert.equal(JSON.parse(listBefore.result.content[0].text).count, 1);

      const imported = await mcp.request('tools/call', {
        name: 'weeklight.import_backup',
        arguments: { filePath: backupFile }
      });
      assert.ok(!imported.result.isError);
      const importedData = JSON.parse(imported.result.content[0].text);
      assert.equal(importedData.imported, 1);
      assert.equal(importedData.total, 1);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: settings read and write', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const get = await mcp.request('tools/call', {
        name: 'weeklight.get_settings',
        arguments: {}
      });
      assert.ok(!get.result.isError);
      const settings = JSON.parse(get.result.content[0].text);
      assert.equal(settings.language, 'zh-CN');

      const updated = await mcp.request('tools/call', {
        name: 'weeklight.update_settings',
        arguments: { language: 'en', sound: false }
      });
      assert.ok(!updated.result.isError);
      const updatedSettings = JSON.parse(updated.result.content[0].text).settings;
      assert.equal(updatedSettings.language, 'en');
      assert.equal(updatedSettings.sound, false);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: invalid task is rejected', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const bad = await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: { title: ' ', date: '2026-02-30', time: '25:00' }
      });
      assert.ok(bad.result.isError, 'should reject invalid task');
      assert.ok(typeof bad.result.content[0].text === 'string');

      const list = await mcp.request('tools/call', {
        name: 'weeklight.list_tasks',
        arguments: {}
      });
      assert.equal(JSON.parse(list.result.content[0].text).count, 0);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: resources and prompts', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const resources = await mcp.request('resources/list', {});
      assert.ok(!resources.error);
      const uris = resources.result.resources.map(r => r.uri);
      assert.ok(uris.includes('weeklight://plans'));
      assert.ok(uris.includes('weeklight://settings'));
      assert.ok(uris.includes('weeklight://reminders'));
      assert.ok(uris.includes('weeklight://data-dir'));

      const plans = await mcp.request('resources/read', { uri: 'weeklight://plans' });
      assert.ok(!plans.error);
      assert.deepEqual(JSON.parse(plans.result.contents[0].text), []);

      const dataDir = await mcp.request('resources/read', { uri: 'weeklight://data-dir' });
      assert.ok(!dataDir.error);
      assert.equal(JSON.parse(dataDir.result.contents[0].text).dataDir, dir);

      const prompts = await mcp.request('prompts/list', {});
      assert.ok(!prompts.error);
      assert.ok(prompts.result.prompts.some(p => p.name === 'weeklight.plan-week'));

      const prompt = await mcp.request('prompts/get', {
        name: 'weeklight.review-tasks',
        arguments: {}
      });
      assert.ok(!prompt.error);
      assert.ok(prompt.result.messages[0].content.text.includes('待办事项回顾'));
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: move, reminder action and due tasks', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const create = await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: {
          title: '即将到期',
          date: '2026-11-10',
          time: '23:59',
          reminder: 0
        }
      });
      assert.ok(!create.result.isError);
      const taskId = JSON.parse(create.result.content[0].text).created[0].id;

      const moved = await mcp.request('tools/call', {
        name: 'weeklight.move_task',
        arguments: { id: taskId, date: '2026-11-12', time: '08:30' }
      });
      assert.ok(!moved.result.isError);
      const movedTask = JSON.parse(moved.result.content[0].text).task;
      assert.equal(movedTask.date, '2026-11-12');
      assert.equal(movedTask.time, '08:30');

      const due = await mcp.request('tools/call', {
        name: 'weeklight.due_tasks',
        arguments: { now: new Date('2026-11-12T08:30:00').getTime() }
      });
      assert.ok(!due.result.isError);
      assert.equal(JSON.parse(due.result.content[0].text).count, 1);

      const snoozed = await mcp.request('tools/call', {
        name: 'weeklight.reminder_action',
        arguments: { id: taskId, action: 'snooze' }
      });
      assert.ok(!snoozed.result.isError);

      const afterSnooze = await mcp.request('tools/call', {
        name: 'weeklight.get_task',
        arguments: { id: taskId }
      });
      assert.ok(!afterSnooze.result.isError);
      assert.ok(JSON.parse(afterSnooze.result.content[0].text).snoozeUntil > 0);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('mcp standalone: following scope completes a whole series', async () => {
  const dir = createTempDir();
  try {
    const mcp = spawnMcp(dir);
    try {
      await init(mcp);

      const create = await mcp.request('tools/call', {
        name: 'weeklight.create_task',
        arguments: {
          title: '每日计划',
          date: '2026-11-10',
          time: '09:00',
          repeat: { frequency: 'daily', until: '2026-11-13' }
        }
      });
      assert.ok(!create.result.isError);
      const tasks = JSON.parse(create.result.content[0].text).created;
      assert.equal(tasks.length, 4);

      const preview = await mcp.request('tools/call', {
        name: 'weeklight.preview_task_action',
        arguments: { id: tasks[1].id, action: 'complete' }
      });
      assert.ok(!preview.result.isError);
      const previewData = JSON.parse(preview.result.content[0].text);
      assert.equal(previewData.source, 'series');
      assert.equal(previewData.followingCount, 3);

      const completed = await mcp.request('tools/call', {
        name: 'weeklight.complete_task',
        arguments: {
          id: tasks[1].id,
          done: true,
          scope: 'following',
          token: previewData.token
        }
      });
      assert.ok(!completed.result.isError, JSON.stringify(completed.result));

      const list = await mcp.request('tools/call', {
        name: 'weeklight.list_tasks',
        arguments: {}
      });
      assert.ok(!list.result.isError);
      const all = JSON.parse(list.result.content[0].text).tasks;
      assert.deepEqual(all.map(t => t.done), [false, true, true, true]);
    } finally {
      mcp.close();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
