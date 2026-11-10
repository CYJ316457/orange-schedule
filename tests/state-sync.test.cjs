const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { signature, createExternalWatcher, createReloader } = require('../src/state-sync.cjs');

function tempFile(content) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'weeklight-sync-'));
  const file = path.join(dir, 'plans.json');
  fs.writeFileSync(file, content, 'utf8');
  return { dir, file };
}

function cleanup(dir) {
  fs.rmSync(dir, { recursive: true, force: true });
}

test('signature is stable for an unchanged file and null for a missing one', () => {
  const { dir, file } = tempFile('{"version":1,"tasks":[]}');
  try {
    assert.equal(typeof signature(file), 'string');
    assert.equal(signature(file), signature(file));
    assert.equal(signature(path.join(dir, 'missing.json')), null);
  } finally {
    cleanup(dir);
  }
});

test('watcher ignores an unchanged file and detects an external write', () => {
  const { dir, file } = tempFile('{"version":1,"tasks":[]}');
  try {
    const watcher = createExternalWatcher(file);
    assert.equal(watcher.check(), false);
    fs.writeFileSync(file, '{"version":1,"tasks":[{"id":"external-1"}]}', 'utf8');
    assert.equal(watcher.check(), true, 'external write must be detected');
    assert.equal(watcher.check(), false, 'the same change must not fire twice');
  } finally {
    cleanup(dir);
  }
});

test('mark() suppresses a change caused by our own persist', () => {
  const { dir, file } = tempFile('{"version":1,"tasks":[]}');
  try {
    const watcher = createExternalWatcher(file);
    fs.writeFileSync(file, '{"version":1,"tasks":[],"settings":{"sound":false}}', 'utf8');
    watcher.mark();
    assert.equal(watcher.check(), false, 'our own write must not trigger a reload');
  } finally {
    cleanup(dir);
  }
});

test('reloader applies the externally written state to the app', () => {
  const { dir, file } = tempFile('{"version":1,"tasks":[]}');
  try {
    const applied = [];
    const errors = [];
    const reloader = createReloader({
      file,
      readState: () => JSON.parse(fs.readFileSync(file, 'utf8')),
      applyState: state => applied.push(state),
      onError: error => errors.push(error)
    });

    assert.equal(reloader.check(), false, 'no reload on startup');
    assert.equal(applied.length, 0);

    const external = { version: 1, tasks: [{ id: 'external-1', title: '外部新增' }] };
    fs.writeFileSync(file, JSON.stringify(external), 'utf8');

    assert.equal(reloader.check(), true, 'external change must reload');
    assert.equal(applied.length, 1, 'state must be replaced exactly once');
    assert.deepEqual(applied[0], external);
    assert.equal(errors.length, 0);

    assert.equal(reloader.check(), false, 'no repeated reload');
    assert.equal(applied.length, 1);
  } finally {
    cleanup(dir);
  }
});

test('reloader keeps the in-memory state when the external file is invalid', () => {
  const { dir, file } = tempFile('{"version":1,"tasks":[]}');
  try {
    const applied = [];
    const errors = [];
    const reloader = createReloader({
      file,
      readState: () => JSON.parse(fs.readFileSync(file, 'utf8')),
      applyState: state => applied.push(state),
      onError: error => errors.push(error)
    });

    assert.equal(reloader.check(), false);
    fs.writeFileSync(file, 'not valid json at all', 'utf8');
    assert.equal(reloader.check(), false, 'a broken file must not replace the state');
    assert.equal(applied.length, 0);
    assert.equal(errors.length, 1);
  } finally {
    cleanup(dir);
  }
});
