// Keep file selection, parsing feedback and error recovery consistent across hosts.
export function installFileOpen({editor, invoke, guard, loadDocument, setState, toast}) {
  const input = document.querySelector('#open-file-input');
  const progress = document.querySelector('#file-progress');
  const status = document.querySelector('#file-progress-text');
  const error = document.querySelector('#file-open-error');
  let opening = false;

  function showError(message) {
    error.querySelector('span').textContent = message;
    error.classList.remove('hidden');
    setState('打开失败');
  }
  error.querySelector('button').onclick = () => error.classList.add('hidden');

  async function run(read, label) {
    if (opening) { toast('正在打开文档，请稍候'); return false; }
    opening = true;
    const editable = editor.isEditable;
    let previousState;
    let locked = false;
    try {
      if (!await guard()) return false;
      previousState = document.querySelector('#save-state').textContent;
      editor.setEditable(false, false);
      locked = true;
      error.classList.add('hidden');
      status.textContent = label;
      progress.classList.remove('hidden');
      progress.focus();
      setState('正在打开文档…');
      // Let the webview paint before a native file dialog or document parsing.
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const result = await read(text => { status.textContent = text; });
      if (!result) { setState(previousState); return false; }
      status.textContent = '正在显示正文…';
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      loadDocument(result);
      toast('文档已打开，可以开始编辑');
      return true;
    } catch (failure) {
      showError(`无法打开文档：${failure.message || String(failure)}`);
      return false;
    } finally {
      if (locked) editor.setEditable(editable, false);
      progress.classList.add('hidden');
      opening = false;
    }
  }

  async function openFiles(files) {
    if (opening) { toast('正在打开文档，请稍候'); return; }
    if (files.length !== 1) { showError('请一次打开一个文档'); return; }
    const file = files[0];
    if (!/\.(docx|txt)$/i.test(file.name)) {
      showError('请选择 .docx 或 UTF-8 .txt 文件，旧版 .doc 请先转换'); return;
    }
    if (file.size > 96 * 1024 * 1024) { showError('导入文件超过 96 MB'); return; }
    if (!file.size) { showError('文件为空，请选择包含内容的文档'); return; }
    return run(async update => {
      const data = await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(',')[1]);
        reader.onerror = () => reject(new Error('文件无法读取，请检查本机文件权限'));
        reader.readAsDataURL(file);
      });
      update('正在解析文档…');
      return invoke('openFile', {name: file.name, data});
    }, `正在读取：${file.name}`);
  }

  input.addEventListener('change', () => {
    const files = Array.from(input.files || []);
    input.value = ''; // Selecting the same file again must still fire change.
    if (files.length) void openFiles(files);
  });
  // Capture document drops before ProseMirror/browser default handling. Images
  // dropped inside the editor retain the existing inline insertion behavior.
  document.addEventListener('dragover', event => {
    if (Array.from(event.dataTransfer?.types || []).includes('Files')) event.preventDefault();
  });
  document.addEventListener('drop', event => {
    const files = Array.from(event.dataTransfer?.files || []);
    if (!files.length) return;
    if (!opening && files.every(file => file.type.startsWith('image/')) && event.target.closest?.('#editor')) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    void openFiles(files);
  }, true);
  document.addEventListener('keydown', event => {
    if (opening && !progress.classList.contains('hidden')) {
      event.preventDefault(); event.stopImmediatePropagation();
    }
  }, true);

  return {
    isOpening: () => opening,
    open() {
      if (opening) { toast('正在打开文档，请稍候'); return; }
      if (window.ipc) return run(() => invoke('open'), '请选择文档，选好后开始读取…');
      // Must run in the click event's user activation, before any async guard.
      input.click();
    },
    openPending() { return run(() => invoke('openPending'), '正在读取文档…'); },
  };
}

// Browser previews download an editable copy instead of opening a native dialog.
export async function saveBrowserCopy(invoke, document, name) {
  const exported = await invoke('exportDocument', {document, name: name.replace(/\.(docx|txt)$/i, '')});
  // Keep a local recovery copy because browsers cannot confirm that the user
  // completed a download after it was started.
  await invoke('autosave', {document, savedAt: new Date().toISOString()});
  const bytes = Uint8Array.from(atob(exported.data), char => char.charCodeAt(0));
  const url = URL.createObjectURL(new Blob([bytes], {type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'}));
  const link = window.document.createElement('a');
  link.href = url;
  link.download = exported.name;
  window.document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
  return {name};
}
