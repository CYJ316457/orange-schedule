const fs = require('node:fs');

function signature(file) {
  try {
    const stat = fs.statSync(file);
    return `${stat.mtimeMs}:${stat.size}`;
  } catch {
    return null;
  }
}

function createExternalWatcher(file) {
  let known = signature(file);
  return {
    check() {
      const current = signature(file);
      const changed = current !== null && current !== known;
      known = current;
      return changed;
    },
    mark() {
      known = signature(file);
    }
  };
}

function createReloader({ file, readState, applyState, onError }) {
  const watcher = createExternalWatcher(file);
  return {
    check() {
      if (!watcher.check()) return false;
      try {
        applyState(readState());
        return true;
      } catch (error) {
        onError?.(error);
        return false;
      }
    },
    mark() {
      watcher.mark();
    }
  };
}

module.exports = { signature, createExternalWatcher, createReloader };
