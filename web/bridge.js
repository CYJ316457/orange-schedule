/*
 * web/bridge.js
 * ------------------------------------------------------------------
 * M0：把 Electron preload 暴露的 window.weeklight 用「内存 mock」实现，
 *     目的只有一个 —— 在没有 Rust 后端的情况下验证 WebView2 的渲染一致性。
 *
 * M1：把 USE_TAURI 改为 true（或删掉 mock 分支），命令名与参数已按
 *     Tauri 的 snake_case 规则写好，业务代码无需改动。
 * ------------------------------------------------------------------
 */
(function () {
  const USE_TAURI = false; // ← M1 改成 true

  // ---------- 真实 Tauri 通道（M1 启用） ----------
  const tauriInvoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
  const tauriListen = (evt, fn) => window.__TAURI__.event.listen(evt, (e) => fn(e.payload));

  // ---------- M0 mock ----------
  const key = (d) => [d.getFullYear(), String(d.getMonth() + 1).padStart(2, '0'), String(d.getDate()).padStart(2, '0')].join('-');
  const add = (d, n) => { const x = new Date(d); x.setDate(x.getDate() + n); return x; };
  const monday = (d) => add(d, -((d.getDay() + 6) % 7));
  const now = new Date();
  const base = monday(now);
  const today = key(now);

  const seed = () => ([
    { id: 'm1', title: '梳理本周优先事项', date: today, time: '09:00', duration: 60, reminder: -1, category: 'work', priority: 'high', notes: '先列清单，再排优先级。', done: false, createdAt: Date.now(), updatedAt: Date.now(), notifiedAt: null, snoozeUntil: null },
    { id: 'm2', title: '留给深度工作的时间', date: key(add(base, 1)), time: '10:00', duration: 120, reminder: -1, category: 'work', priority: 'normal', notes: '', done: false, createdAt: Date.now(), updatedAt: Date.now(), notifiedAt: null, snoozeUntil: null },
    { id: 'm3', title: '读完一章，记下想法', date: key(add(base, 2)), time: '19:30', duration: 45, reminder: -1, category: 'study', priority: 'normal', notes: '', done: false, createdAt: Date.now(), updatedAt: Date.now(), notifiedAt: null, snoozeUntil: null },
    { id: 'm4', title: '散步，给自己充充电', date: key(add(base, 3)), time: '18:00', duration: 30, reminder: -1, category: 'life', priority: 'normal', notes: '', done: true, createdAt: Date.now(), updatedAt: Date.now(), notifiedAt: null, snoozeUntil: null },
    { id: 'm5', title: '整理项目进展', date: key(add(base, 4)), time: '14:00', duration: 60, reminder: -1, category: 'work', priority: 'normal', notes: '', done: false, createdAt: Date.now(), updatedAt: Date.now(), notifiedAt: null, snoozeUntil: null },
    { id: 'm6', title: '一周回顾与下周计划', date: key(add(base, 6)), time: '17:00', duration: 30, reminder: -1, category: 'life', priority: 'normal', notes: '', done: false, createdAt: Date.now(), updatedAt: Date.now(), notifiedAt: null, snoozeUntil: null },
  ]);

  const defaultSettings = () => ({
    language: 'zh-CN', sound: true, popup: true, checkUpdates: true,
    widget: { enabled: false, mode: 'day', anchor: null, category: 'all', onTop: false, theme: 'sage', showCompleted: true, bounds: null },
    reminderRotation: { seen: [], currentId: null },
  });

  let mock = null;
  const listeners = { state: [], new: [], edit: [], error: [] };
  const emit = (evt, payload) => listeners[evt].forEach((fn) => { try { fn(payload); } catch (e) { console.error(e); } });

  function loadMock() {
    if (mock) return mock;
    let saved = null;
    try { saved = JSON.parse(localStorage.getItem('weeklight.mock') || 'null'); } catch (_) {}
    mock = {
      version: 1,
      tasks: saved?.tasks?.length ? saved.tasks : seed(),
      settings: Object.assign(defaultSettings(), saved?.settings || {}),
      history: saved?.history || [],
    };
    return mock;
  }
  function saveMock() {
    try {
      localStorage.setItem('weeklight.mock', JSON.stringify({ tasks: mock.tasks, settings: mock.settings, history: mock.history }));
    } catch (_) {}
  }

  function snapshot() {
    const s = loadMock();
    return {
      version: 1,
      tasks: s.tasks,
      settings: s.settings,
      history: s.history,
      update: { phase: 'idle', currentVersion: '1.2.1', release: null, checkedAt: null, error: '', dismissed: false },
      dataDir: '(M0 mock) %APPDATA%\\weeklight',
      startupError: '',
      autostart: false,
      autostartError: '',
      packaged: false,
      reminderMessage: { id: 'mock-quote', text: '兴尽晚回舟，误入藕花深处。', source: '《如梦令·常记溪亭日暮》' },
      reminderMessageCount: 120,
    };
  }

  const uid = () => 'm' + Math.random().toString(36).slice(2, 10);
  const commit = (fn) => { const s = loadMock(); fn(s); saveMock(); const snap = snapshot(); emit('state', snap); return snap; };

  const commands = {
    get: () => snapshot(),
    saveTask: (task) => commit((s) => {
      const id = task.id || uid();
      const row = {
        id, title: String(task.title || '').trim() || '未命名计划',
        date: task.date || today, time: task.time || '09:00',
        duration: Number(task.duration) || 60, reminder: Number(task.reminder ?? -1),
        category: ['work', 'study', 'life'].includes(task.category) ? task.category : 'work',
        priority: task.priority === 'high' ? 'high' : 'normal',
        notes: String(task.notes || ''), done: !!task.done,
        createdAt: Date.now(), updatedAt: Date.now(), notifiedAt: null, snoozeUntil: null,
      };
      const i = s.tasks.findIndex((x) => x.id === id);
      if (i >= 0) s.tasks[i] = row; else s.tasks.push(row);
    }),
    previewTaskAction: () => ({ ok: true }),
    applyTaskAction: () => commit(() => {}),
    toggleTask: (id) => commit((s) => { const t = s.tasks.find((x) => x.id === id); if (t) t.done = !t.done; }),
    deleteTask: (id) => commit((s) => { s.tasks = s.tasks.filter((x) => x.id !== id); }),
    moveTask: (id, date, time) => commit((s) => { const t = s.tasks.find((x) => x.id === id); if (t) { t.date = date; t.time = time || t.time; } }),
    reminderAction: (id, action) => commit((s) => { const t = s.tasks.find((x) => x.id === id); if (t && action === 'done') t.done = true; }),
    settings: (patch) => commit((s) => { Object.assign(s.settings, patch || {}); }),
    widgetSettings: (patch) => commit((s) => { Object.assign(s.settings.widget, patch || {}); }),
    checkUpdates: () => snapshot().update,
    dismissUpdate: () => snapshot().update,
    downloadUpdate: () => true,
    openUpdatePage: () => true,
    testReminder: () => true,
    nextReminderMessage: () => snapshot(),
    exportData: () => null,
    importData: () => null,
    openData: () => true,
    showMain: () => true,
    openWidget: () => snapshot(),
    closeWidget: () => snapshot(),
    editTask: (id) => { emit('edit', id); return true; },
    quit: () => true,
  };

  // ---------- 对外接口（与 Electron preload 完全同名） ----------
  window.weeklight = USE_TAURI ? {
    get: () => tauriInvoke('get'),
    saveTask: (task, repeat) => tauriInvoke('save_task', { task, repeat }),
    previewTaskAction: (id, action) => tauriInvoke('preview_task_action', { id, action }),
    applyTaskAction: (request) => tauriInvoke('apply_task_action', { request }),
    toggleTask: (id) => tauriInvoke('toggle_task', { id }),
    deleteTask: (id) => tauriInvoke('delete_task', { id }),
    moveTask: (id, date, time) => tauriInvoke('move_task', { id, date, time }),
    reminderAction: (id, action) => tauriInvoke('reminder_action', { id, action }),
    settings: (s) => tauriInvoke('settings', { settings: s }),
    checkUpdates: () => tauriInvoke('check_updates'),
    dismissUpdate: () => tauriInvoke('dismiss_update'),
    downloadUpdate: () => tauriInvoke('download_update'),
    openUpdatePage: () => tauriInvoke('open_update_page'),
    testReminder: () => tauriInvoke('test_reminder'),
    nextReminderMessage: () => tauriInvoke('next_reminder_message'),
    exportData: () => tauriInvoke('export_data'),
    importData: () => tauriInvoke('import_data'),
    openData: () => tauriInvoke('open_data'),
    openWidget: () => tauriInvoke('open_widget'),
    closeWidget: () => tauriInvoke('close_widget'),
    widgetSettings: (p) => tauriInvoke('widget_settings', { patch: p }),
    editTask: (id) => tauriInvoke('edit_task', { id }),
    showMain: () => tauriInvoke('show_main'),
    quit: () => tauriInvoke('quit'),
    onState: (fn) => tauriListen('state', fn),
    onNew: (fn) => tauriListen('new-task', fn),
    onEditTask: (fn) => tauriListen('edit-task', fn),
    onError: (fn) => tauriListen('app-error', fn),
  } : {
    get: () => Promise.resolve().then(() => commands.get()),
    saveTask: (task, repeat) => Promise.resolve().then(() => commands.saveTask(task, repeat)),
    previewTaskAction: (id, action) => Promise.resolve().then(() => commands.previewTaskAction(id, action)),
    applyTaskAction: (request) => Promise.resolve().then(() => commands.applyTaskAction(request)),
    toggleTask: (id) => Promise.resolve().then(() => commands.toggleTask(id)),
    deleteTask: (id) => Promise.resolve().then(() => commands.deleteTask(id)),
    moveTask: (id, date, time) => Promise.resolve().then(() => commands.moveTask(id, date, time)),
    reminderAction: (id, action) => Promise.resolve().then(() => commands.reminderAction(id, action)),
    settings: (s) => Promise.resolve().then(() => commands.settings(s)),
    checkUpdates: () => Promise.resolve().then(() => commands.checkUpdates()),
    dismissUpdate: () => Promise.resolve().then(() => commands.dismissUpdate()),
    downloadUpdate: () => Promise.resolve().then(() => commands.downloadUpdate()),
    openUpdatePage: () => Promise.resolve().then(() => commands.openUpdatePage()),
    testReminder: () => Promise.resolve().then(() => commands.testReminder()),
    nextReminderMessage: () => Promise.resolve().then(() => commands.nextReminderMessage()),
    exportData: () => Promise.resolve().then(() => commands.exportData()),
    importData: () => Promise.resolve().then(() => commands.importData()),
    openData: () => Promise.resolve().then(() => commands.openData()),
    openWidget: () => Promise.resolve().then(() => commands.openWidget()),
    closeWidget: () => Promise.resolve().then(() => commands.closeWidget()),
    widgetSettings: (p) => Promise.resolve().then(() => commands.widgetSettings(p)),
    editTask: (id) => Promise.resolve().then(() => commands.editTask(id)),
    showMain: () => Promise.resolve().then(() => commands.showMain()),
    quit: () => Promise.resolve().then(() => commands.quit()),
    onState: (fn) => { listeners.state.push(fn); },
    onNew: (fn) => { listeners.new.push(fn); },
    onEditTask: (fn) => { listeners.edit.push(fn); },
    onError: (fn) => { listeners.error.push(fn); },
  };

  if (!USE_TAURI) console.info('[bridge] M0 mock 模式：数据仅存在 localStorage');
})();
