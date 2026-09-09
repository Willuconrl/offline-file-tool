/* 文件批量处理工具箱 —— 前端逻辑（纯本地处理，不联网） */
(function () {
  'use strict';

  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const { t, applyI18n, setLang, lang } = window.I18N;

  // ---------- 全局状态 ----------
  const state = {
    tool: 'rename',
    busy: false,
    currentJob: null,
    rename: {
      files: [], // {path, name, dir, size}
      preview: [],
      previewSeq: 0,
    },
    images: {
      files: [], // {path, name, size, width, height, format, color, thumb, error}
      selected: null,
      outDir: '', // '' = 原目录
      results: null,
    },
    pdf: {
      mode: 'merge',
      mergeFiles: [], // {path, name, size, pages, encrypted, error}
      outName: 'merged.pdf',
      outPath: '', // 保存路径（对话框选择）
      source: null, // 拆分源文件
      splitMode: 'ranges',
      outDir: '',
      parts: [],
      results: null,
    },
    wm: {
      files: [], // {path, name, size}
      layers: [], // 水印图层列表
      selLayer: -1, // 当前编辑的图层索引
      outDir: '',
      results: null,
      previewDataUrl: null,
    },
  };

  // ---------- 工具函数 ----------
  const $ = (id) => document.getElementById(id);

  function esc(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({
      '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
    }[c]));
  }

  function fmtSize(bytes) {
    if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
    if (bytes < 1024) return bytes + ' B';
    const units = ['KB', 'MB', 'GB'];
    let v = bytes;
    let i = -1;
    do { v /= 1024; i++; } while (v >= 1024 && i < units.length - 1);
    return v.toFixed(v >= 100 ? 0 : 1) + ' ' + units[i];
  }

  function baseName(p) {
    return p.split(/[\\/]/).pop() || p;
  }

  function dirName(p) {
    const i = Math.max(p.lastIndexOf('\\'), p.lastIndexOf('/'));
    return i > 0 ? p.slice(0, i) : '';
  }

  function debounce(fn, ms) {
    let t = null;
    return (...args) => {
      clearTimeout(t);
      t = setTimeout(() => fn(...args), ms);
    };
  }

  // 后端错误码 → 本地化文案。格式：@code|param1|param2（最后一段可含 |）
  const ERR_PARAMS = {
    err_rename_failed: ['msg'],
    err_img_read: ['msg'],
    err_img_too_large: ['w', 'h'],
    err_encode_jpeg: ['msg'],
    err_encode_webp: ['msg'],
    err_encode_png: ['msg'],
    err_encode_fmt: ['fmt'],
    err_encode: ['msg'],
    err_write: ['msg'],
    err_pdf_read: ['msg'],
    err_pdf_read_file: ['name', 'msg'],
    err_pdf_encrypted_merge: ['name'],
    err_page_out_of_range: ['page', 'total'],
    err_page_tree: ['msg'],
    err_pdf_write: ['msg'],
    err_open_explorer: ['msg'],
    err_wm_no_font: ['msg'],
    err_wm_img_read: ['msg'],
    msg_reading_file: ['name'],
  };

  function localizeError(raw) {
    if (typeof raw !== 'string' || raw.charAt(0) !== '@') return String(raw);
    const seg = raw.slice(1).split('|');
    const code = seg[0];
    const names = ERR_PARAMS[code] || [];
    const params = {};
    for (let i = 0; i < names.length; i++) {
      if (i === names.length - 1 && seg.length - 1 > names.length) {
        params[names[i]] = seg.slice(i + 1).join('|');
      } else {
        params[names[i]] = seg[i + 1] !== undefined ? seg[i + 1] : '';
      }
    }
    // 字典键即错误码本身（err_xxx / msg_xxx 完整前缀）
    return t(code, params);
  }

  let statusColorTimer = null;

  function setStatus(msg, isError) {
    const el = $('status-msg');
    el.textContent = msg;
    if (statusColorTimer) {
      clearTimeout(statusColorTimer);
      statusColorTimer = null;
    }
    el.style.color = isError ? 'var(--danger)' : '';
    if (!isError) {
      statusColorTimer = setTimeout(() => {
        statusColorTimer = null;
        el.style.color = '';
      }, 6000);
    }
  }

  let progressHideTimer = null;

  function setBusy(busy, msg) {
    state.busy = busy;
    if (progressHideTimer) {
      clearTimeout(progressHideTimer);
      progressHideTimer = null;
    }
    if (busy) {
      $('progress-wrap').classList.remove('hidden');
      $('progress-bar').style.width = '0%';
      if (msg) setStatus(msg);
    } else {
      progressHideTimer = setTimeout(() => {
        progressHideTimer = null;
        $('progress-wrap').classList.add('hidden');
      }, 400);
    }
    refreshButtons();
  }

  function onJobProgress(p) {
    if (!p || p.jobId !== state.currentJob) return;
    $('progress-wrap').classList.remove('hidden');
    $('progress-bar').style.width =
      p.total > 0 ? Math.round((p.current / p.total) * 100) + '%' : '0%';
    if (p.message) setStatus(localizeError(p.message));
    if (p.done) {
      state.currentJob = null;
      if (progressHideTimer) {
        clearTimeout(progressHideTimer);
        progressHideTimer = null;
      }
      progressHideTimer = setTimeout(() => {
        progressHideTimer = null;
        $('progress-wrap').classList.add('hidden');
      }, 500);
    }
  }

  async function call(cmd, args, busyMsg) {
    if (state.busy) {
      setStatus(t('busy_msg'), true);
      throw new Error(t('busy_msg'));
    }
    if (busyMsg) setBusy(true, busyMsg);
    try {
      return await invoke(cmd, args || {});
    } catch (e) {
      setStatus(t('op_failed', { msg: localizeError(e) }), true);
      throw e;
    } finally {
      if (busyMsg) {
        setBusy(false);
        refreshButtons();
      }
    }
  }

  // ---------- 导航 ----------
  function switchTool(tool) {
    state.tool = tool;
    document.querySelectorAll('.nav-item').forEach((b) =>
      b.classList.toggle('active', b.dataset.tool === tool));
    document.querySelectorAll('.tool-panel').forEach((p) =>
      p.classList.add('hidden'));
    $('panel-' + tool).classList.remove('hidden');
    renderPreview();
  }

  document.querySelectorAll('.nav-item').forEach((b) =>
    b.addEventListener('click', () => switchTool(b.dataset.tool)));

  function refreshButtons() {
    const busy = state.busy;
    $('rename-add').disabled = busy;
    $('rename-apply').disabled = busy || state.rename.files.length === 0;
    $('rename-apply').title = busy ? t('hint_busy')
      : state.rename.files.length === 0 ? t('hint_add_files') : t('hint_rename_go');
    $('img-add').disabled = busy;
    const imgReady = state.images.files.filter((f) => !f.error).length;
    $('img-run').disabled = busy || imgReady === 0;
    $('img-run').title = busy ? t('hint_busy')
      : imgReady === 0
        ? (state.images.files.length ? t('hint_img_unreadable') : t('hint_img_add'))
        : t('hint_img_go');
    $('pdf-add').disabled = busy;
    const mergeReady = state.pdf.mergeFiles.filter((f) => !f.error).length;
    $('pdf-merge-run').disabled = busy || mergeReady < 2 || !state.pdf.outPath;
    $('pdf-merge-run').title = busy ? t('hint_busy')
      : mergeReady < 2 ? t('hint_merge_need2') : !state.pdf.outPath ? t('hint_pick_save') : t('hint_merge_go');
    $('pdf-src-pick').disabled = busy;
    const splitReady = state.pdf.source && !state.pdf.source.error && state.pdf.parts.length > 0 && !state.pdf.partsError;
    $('pdf-split-run').disabled = busy || !splitReady;
    $('pdf-split-run').title = busy ? t('hint_busy')
      : !state.pdf.source ? t('hint_pick_pdf')
        : state.pdf.partsError ? t('hint_split_plan', { msg: state.pdf.partsError })
          : state.pdf.parts.length === 0 ? t('hint_split_empty') : t('hint_split_go');
    $('wm-add').disabled = busy;
    $('wm-run').disabled = busy || state.wm.files.length === 0 || state.wm.layers.length === 0;
    $('wm-run').title = busy ? t('hint_busy')
      : state.wm.files.length === 0 ? t('hint_wm_add')
        : state.wm.layers.length === 0 ? t('wm_no_layers') : t('hint_wm_go');
  }

  // ============================================================
  // 批量重命名
  // ============================================================
  function renameRules() {
    return {
      template: $('r-template').value,
      numStart: parseInt($('r-num-start').value, 10) || 1,
      numStep: parseInt($('r-num-step').value, 10) || 1,
      dateFormat: $('r-date-fmt').value,
      find: $('r-find').value,
      replace: $('r-replace').value,
      caseSensitive: $('r-case').checked,
      caseMode: $('r-case-mode').value,
      spaceMode: $('r-space-mode').value,
    };
  }

  // 在模板输入框光标处插入占位符
  function insertToken(token) {
    const input = $('r-template');
    const s = input.selectionStart ?? input.value.length;
    const e = input.selectionEnd ?? s;
    input.value = input.value.slice(0, s) + token + input.value.slice(e);
    input.focus();
    const pos = s + token.length;
    input.setSelectionRange(pos, pos);
    scheduleRenamePreview();
  }

  // 一键模板
  function applyPreset(tpl) {
    $('r-template').value = tpl;
    $('r-template').focus();
    scheduleRenamePreview();
  }

  function renderRenameList() {
    const box = $('rename-list');
    $('rename-count').textContent = t('count_files', { n: state.rename.files.length });
    if (state.rename.files.length === 0) {
      box.innerHTML = `<div class="empty">${esc(t('empty_rename'))}</div>`;
      return;
    }
    box.innerHTML = state.rename.files.map((f, i) => `
      <div class="file-item">
        <span class="fname" title="${esc(f.path)}">${esc(f.name)}</span>
        <span class="fmeta">${fmtSize(f.size)} · ${esc(f.dir)}</span>
        <button class="icon-btn" data-remove="${i}" title="${esc(t('btn_remove'))}">✕</button>
      </div>`).join('');
    box.querySelectorAll('[data-remove]').forEach((b) =>
      b.addEventListener('click', () => {
        if (state.busy) return;
        state.rename.files.splice(Number(b.dataset.remove), 1);
        renderRenameList();
        scheduleRenamePreview();
        refreshButtons();
      }));
  }

  const scheduleRenamePreview = debounce(updateRenamePreview, 250);

  async function updateRenamePreview() {
    if (state.tool !== 'rename') return;
    if (state.rename.files.length === 0) {
      state.rename.preview = [];
      renderPreview();
      return;
    }
    // 请求序号：丢弃过期响应，避免旧预览覆盖新状态（与「开始重命名」交错的竞态）
    const seq = ++state.previewSeq;
    try {
      const res = await invoke('preview_rename', {
        paths: state.rename.files.map((f) => f.path),
        rules: renameRules(),
      });
      if (seq !== state.previewSeq) return; // 已有更新的请求
      state.rename.preview = res.entries || [];
      renderPreview();
    } catch (e) {
      /* 状态栏已提示 */
    }
  }

  async function renameAddFiles() {
    try {
      const picked = await call('pick_files', { filters: [[t('filter_all_files'), ['*']]] }, t('status_picking_files'));
      if (!picked.length) return;
      const seen = new Set(state.rename.files.map((f) => f.path.toLowerCase()));
      for (const p of picked) {
        if (seen.has(p.toLowerCase())) continue;
        seen.add(p.toLowerCase());
        const meta = await metaOf(p);
        state.rename.files.push({ path: p, name: meta.name, dir: dirName(p), size: meta.size });
      }
      renderRenameList();
      scheduleRenamePreview();
      refreshButtons();
    } catch (e) { /* 已提示 */ }
  }

  async function metaOf(p) {
    try {
      const res = await invoke('file_meta', { path: p });
      return res;
    } catch (e) {
      return { name: baseName(p), size: 0 };
    }
  }

  async function renameApply() {
    if (state.busy) return;
    const hasConflicts = state.rename.preview.some((e) => e.conflict);
    const resolve = $('rename-resolve').checked;
    if (hasConflicts && !resolve) {
      if (!confirm(t('confirm_apply_conflicts'))) return;
    }
    const files = state.rename.files.slice();
    setBusy(true, t('status_renaming'));
    try {
      const results = await invoke('apply_rename', {
        paths: files.map((f) => f.path),
        rules: renameRules(),
        resolveConflicts: resolve,
      });
      const ok = results.filter((r) => r.ok).length;
      const fail = results.length - ok;
      // 更新列表路径
      const map = new Map(results.map((r) => [(r.path || '').toLowerCase(), r]));
      state.rename.files = state.rename.files.map((f) => {
        const r = map.get((f.path || '').toLowerCase());
        if (r && r.ok && r.newName) {
          return { ...f, path: f.dir + '\\' + r.newName, name: r.newName };
        }
        return f;
      });
      renderRenameList();
      await updateRenamePreview();
      setStatus(t('status_rename_done', { ok }) + (fail ? t('status_rename_done_fail', { n: fail }) : ''), fail > 0);
    } catch (e) {
      setStatus(t('op_failed', { msg: localizeError(e) }), true);
    } finally {
      setBusy(false);
      refreshButtons();
    }
  }

  // ============================================================
  // 图片压缩 / 转换
  // ============================================================
  function renderImageList() {
    const box = $('img-list');
    const files = state.images.files;
    $('img-count').textContent = t('count_images', { n: files.length });
    if (files.length === 0) {
      box.innerHTML = `<div class="empty wide">${esc(t('empty_images'))}</div>`;
      return;
    }
    box.innerHTML = files.map((f, i) => `
      <div class="img-card ${f.error ? 'bad' : ''} ${state.images.selected === i ? 'selected' : ''}" data-i="${i}">
        <div class="thumb-wrap">
          ${f.thumb ? `<img src="${f.thumb}" alt="" loading="lazy" />` : ''}
        </div>
        <div class="info">
          <div class="n" title="${esc(f.path)}">${esc(f.name)}</div>
          ${f.error
            ? `<div class="err-badge">${esc(t('badge_unreadable'))}</div>`
            : `<div class="m">${f.width}×${f.height} · ${esc(f.format)} · ${fmtSize(f.size)}</div>`}
        </div>
      </div>`).join('');
    box.querySelectorAll('.img-card[data-i]').forEach((c) =>
      c.addEventListener('click', () => {
        state.images.selected = Number(c.dataset.i);
        renderImageList();
        renderPreview();
      }));
  }

  async function imgAddFiles() {
    try {
      const picked = await call('pick_files', {
        filters: [[t('filter_images'), ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif', 'heic', 'heif']]],
      }, t('status_picking_files'));
      if (!picked.length) return;
      const seen = new Set(state.images.files.map((f) => f.path.toLowerCase()));
      const fresh = picked.filter((p) => !seen.has(p.toLowerCase()));
      if (!fresh.length) return;
      const metas = await call('image_meta', { paths: fresh }, t('status_reading'));
      state.images.files.push(...metas);
      state.images.results = null; // 新文件加入后旧结果不再有意义
      renderImageList();
      renderPreview();
      refreshButtons();
    } catch (e) { /* 已提示 */ }
  }

  function imgOptions() {
    return {
      outFormat: $('img-format').value,
      quality: parseInt($('img-quality').value, 10) || 82,
      resize: $('img-resize-on').checked,
      resizeMode: $('img-resize-mode').value,
      percent: parseInt($('img-resize-val').value, 10) || 80,
      maxSide: parseInt($('img-resize-val').value, 10) || 1920,
      overwrite: false,
    };
  }

  async function imgRun() {
    if (state.busy) return;
    const files = state.images.files.filter((f) => !f.error);
    if (!files.length) return;
    const jobId = 'job-' + Date.now();
    state.currentJob = jobId;
    const outDir = state.images.outDir || null;
    setBusy(true, t('status_processing'));
    try {
      const results = await invoke('process_images', {
        jobId,
        paths: files.map((f) => f.path),
        options: imgOptions(),
        outDir,
      });
      state.images.results = results;
      const ok = results.filter((r) => r.ok);
      const inTotal = ok.reduce((s, r) => s + r.inSize, 0);
      const outTotal = ok.reduce((s, r) => s + (r.outSize || 0), 0);
      const saved = inTotal > 0 ? ((inTotal - outTotal) / inTotal * 100) : 0;
      let msg = t('status_images_done', { ok: ok.length, total: results.length });
      if (inTotal) {
        msg += t('size_change', { sign: saved > 0 ? '-' : '+', pct: Math.abs(saved).toFixed(1) });
      }
      setStatus(msg, results.length > ok.length);
      renderPreview();
    } catch (e) {
      setStatus(t('op_failed', { msg: localizeError(e) }), true);
    } finally {
      setBusy(false);
      refreshButtons();
    }
  }

  // ============================================================
  // PDF 合并 / 拆分
  // ============================================================
  function switchPdfMode(mode) {
    state.pdf.mode = mode;
    document.querySelectorAll('#pdf-mode-switch button').forEach((b) =>
      b.classList.toggle('active', b.dataset.mode === mode));
    $('pdf-merge-panel').classList.toggle('hidden', mode !== 'merge');
    $('pdf-split-panel').classList.toggle('hidden', mode !== 'split');
    renderPreview();
  }

  function renderMergeList() {
    const box = $('pdf-merge-list');
    const files = state.pdf.mergeFiles;
    $('pdf-count').textContent = t('count_pdf', { n: files.length });
    if (files.length === 0) {
      box.innerHTML = `<div class="empty">${esc(t('empty_pdf'))}</div>`;
      return;
    }
    box.innerHTML = files.map((f, i) => `
      <div class="file-item">
        <span class="fmeta">${i + 1}</span>
        <span class="fname" title="${esc(f.path)}">${esc(f.name)}</span>
        <span class="fmeta">${f.error ? esc(localizeError(f.error)) : t('pages_fmt', { n: f.pages }) + ' · ' + fmtSize(f.size)}</span>
        <button class="icon-btn" data-up="${i}" ${i === 0 ? 'disabled' : ''} title="${esc(t('btn_up'))}">↑</button>
        <button class="icon-btn" data-down="${i}" ${i === files.length - 1 ? 'disabled' : ''} title="${esc(t('btn_down'))}">↓</button>
        <button class="icon-btn" data-rm="${i}" title="${esc(t('btn_remove'))}">✕</button>
      </div>`).join('');
    box.querySelectorAll('[data-up]').forEach((b) => b.addEventListener('click', () => {
      if (state.busy) return;
      const i = Number(b.dataset.up);
      [state.pdf.mergeFiles[i - 1], state.pdf.mergeFiles[i]] = [state.pdf.mergeFiles[i], state.pdf.mergeFiles[i - 1]];
      renderMergeList(); renderPreview();
    }));
    box.querySelectorAll('[data-down]').forEach((b) => b.addEventListener('click', () => {
      if (state.busy) return;
      const i = Number(b.dataset.down);
      [state.pdf.mergeFiles[i + 1], state.pdf.mergeFiles[i]] = [state.pdf.mergeFiles[i], state.pdf.mergeFiles[i + 1]];
      renderMergeList(); renderPreview();
    }));
    box.querySelectorAll('[data-rm]').forEach((b) => b.addEventListener('click', () => {
      if (state.busy) return;
      state.pdf.mergeFiles.splice(Number(b.dataset.rm), 1);
      renderMergeList(); renderPreview();
      refreshButtons();
    }));
  }

  async function pdfAddFiles() {
    try {
      const picked = await call('pick_files', { filters: [[t('filter_pdf'), ['pdf']]] }, t('status_picking_files'));
      if (!picked.length) return;
      const seen = new Set(state.pdf.mergeFiles.map((f) => f.path.toLowerCase()));
      const fresh = picked.filter((p) => !seen.has(p.toLowerCase()));
      if (!fresh.length) return;
      const metas = await call('pdf_meta', { paths: fresh }, t('status_reading_pdf'));
      state.pdf.mergeFiles.push(...metas);
      state.pdf.results = null; // 新文件加入后旧结果不再有意义
      if (!state.pdf.outPath && state.pdf.mergeFiles.length) {
        state.pdf.outPath = dirName(state.pdf.mergeFiles[0].path) + '\\' + $('pdf-out-name').value;
      }
      renderMergeList();
      renderPreview();
      refreshButtons();
    } catch (e) { /* 已提示 */ }
  }

  async function pdfPickSave() {
    try {
      const def = state.pdf.mergeFiles.length ? dirName(state.pdf.mergeFiles[0].path) : null;
      const path = await call('pick_save_path', {
        defaultName: $('pdf-out-name').value,
        defaultDir: def,
      }, t('status_picking_files'));
      if (path) {
        state.pdf.outPath = path;
        renderPreview();
        refreshButtons();
      }
    } catch (e) { /* 已提示 */ }
  }

  async function pdfMergeRun() {
    if (state.busy) return;
    const files = state.pdf.mergeFiles.filter((f) => !f.error);
    if (files.length < 2 || !state.pdf.outPath) return;
    const jobId = 'job-' + Date.now();
    state.currentJob = jobId;
    setBusy(true, t('status_merging'));
    try {
      const res = await invoke('pdf_merge', {
        jobId,
        paths: files.map((f) => f.path),
        outPath: state.pdf.outPath,
      });
      state.pdf.results = { type: 'merge', ...res };
      setStatus(t('status_merge_done', { n: res.pages, size: fmtSize(res.size) }));
      renderPreview();
    } catch (e) {
      setStatus(t('op_failed', { msg: localizeError(e) }), true);
    } finally {
      setBusy(false);
      refreshButtons();
    }
  }

  async function pdfPickSource() {
    try {
      const picked = await call('pick_files', { filters: [[t('filter_pdf'), ['pdf']]] }, t('status_picking_files'));
      if (!picked.length) return;
      const metas = await call('pdf_meta', { paths: [picked[0]] }, t('status_reading_pdf'));
      state.pdf.source = metas[0];
      state.pdf.results = null; // 换源后旧拆分结果不再有意义
      if (!state.pdf.outDir) state.pdf.outDir = dirName(picked[0]);
      renderSplitSource();
      computeParts();
      renderPreview();
      refreshButtons();
    } catch (e) { /* 已提示 */ }
  }

  function renderSplitSource() {
    const box = $('pdf-src-info');
    const s = state.pdf.source;
    if (!s) {
      box.innerHTML = `<div class="empty wide">${esc(t('empty_source'))}</div>`;
      return;
    }
    box.innerHTML = `
      <div class="src-file">
        <span class="fname" title="${esc(s.path)}">${esc(s.name)}</span>
        <span class="fmeta">${s.error ? esc(localizeError(s.error)) : t('pages_fmt', { n: s.pages }) + ' · ' + fmtSize(s.size)}</span>
      </div>`;
  }

  function parseRanges(text, pageTotal) {
    const parts = [];
    for (const raw of String(text).split(/[,，;；]/)) {
      const tok = raw.trim();
      if (!tok) continue;
      const m = tok.match(/^(\d+)\s*-\s*(\d+)$/);
      if (m) {
        let a = parseInt(m[1], 10), b = parseInt(m[2], 10);
        if (a > b) [a, b] = [b, a];
        parts.push([a, b]);
      } else if (/^\d+$/.test(tok)) {
        const a = parseInt(tok, 10);
        parts.push([a, a]);
      } else {
        throw new Error(t('err_parse_range', { t: tok }));
      }
    }
    if (!parts.length) throw new Error(t('err_no_ranges'));
    for (const [a, b] of parts) {
      if (a < 1 || b > pageTotal) throw new Error(t('err_range_out', { a, b, n: pageTotal }));
    }
    return parts;
  }

  function computeParts() {
    const s = state.pdf.source;
    state.pdf.parts = [];
    if (!s || s.error || !s.pages) return;
    const base = (s.name || 'document').replace(/\.pdf$/i, '');
    try {
      if (state.pdf.splitMode === 'ranges') {
        const ranges = parseRanges($('pdf-ranges').value, s.pages);
        const usedNames = new Set();
        state.pdf.parts = ranges.map(([a, b]) => {
          const label = `${a}${b !== a ? '-' + b : ''}`;
          let name = `${base}_p${label}.pdf`;
          let k = 1;
          while (usedNames.has(name.toLowerCase())) {
            name = `${base}_p${label} (${++k}).pdf`;
          }
          usedNames.add(name.toLowerCase());
          return {
            name,
            pages: Array.from({ length: b - a + 1 }, (_, i) => a + i),
          };
        });
      } else {
        const n = Math.max(1, parseInt($('pdf-every').value, 10) || 1);
        let k = 1;
        for (let start = 1; start <= s.pages; start += n) {
          const end = Math.min(start + n - 1, s.pages);
          state.pdf.parts.push({
            name: `${base}_part${k++}.pdf`,
            pages: Array.from({ length: end - start + 1 }, (_, i) => start + i),
          });
        }
      }
      state.pdf.partsError = '';
    } catch (e) {
      state.pdf.partsError = e.message;
      state.pdf.parts = [];
    }
  }

  async function pdfPickOutDir() {
    try {
      const dir = await call('pick_folder', {}, t('status_picking_dir'));
      if (dir) {
        state.pdf.outDir = dir;
        $('pdf-split-outdir').textContent = dir;
        renderPreview();
      }
    } catch (e) { /* 已提示 */ }
  }

  async function pdfSplitRun() {
    if (state.busy) return;
    if (!state.pdf.source || !state.pdf.parts.length) return;
    const jobId = 'job-' + Date.now();
    state.currentJob = jobId;
    setBusy(true, t('status_splitting'));
    try {
      const results = await invoke('pdf_split', {
        jobId,
        path: state.pdf.source.path,
        parts: state.pdf.parts,
        outDir: state.pdf.outDir,
      });
      state.pdf.results = { type: 'split', list: results };
      const ok = results.filter((r) => r.ok).length;
      setStatus(t('status_split_done', { ok, total: results.length }), results.length > ok);
      renderPreview();
    } catch (e) { /* 已提示 */ } finally {
      setBusy(false);
      refreshButtons();
    }
  }

  // ============================================================
  // 水印（多图层）
  // ============================================================
  function newLayer(kind) {
    state.wm.layers.push({
      kind,
      text: '',
      fontSize: 48,
      color: '#808080',
      opacity: 40,
      rotation: 0,
      position: 'center',
      margin: 24,
      tileGap: 60,
      imagePath: null,
      imageScale: 40,
      count: 1,
    });
    selectLayer(state.wm.layers.length - 1);
  }

  function renderWmLayers() {
    const box = $('wm-layers');
    const layers = state.wm.layers;
    if (!layers.length) {
      box.innerHTML = `<div class="empty">${esc(t('wm_no_layers'))}</div>`;
      $('wm-editor-card').classList.add('hidden');
      $('wm-layer-badge').textContent = '';
      return;
    }
    $('wm-editor-card').classList.remove('hidden');
    box.innerHTML = layers.map((l, i) => `
      <div class="wm-layer-item${i === state.wm.selLayer ? ' selected' : ''}" data-idx="${i}">
        <span class="li-kind">${esc(l.kind === 'image' ? t('wm_layer_image') : t('wm_layer_text'))}</span>
        <span class="li-desc">${esc(l.kind === 'image'
          ? (l.imagePath ? baseName(l.imagePath) : '—')
          : (l.text || '—'))}</span>
        <button class="icon-btn" data-remove-layer="${i}" title="${esc(t('wm_remove_layer'))}">✕</button>
      </div>`).join('');
    box.querySelectorAll('.wm-layer-item').forEach((el) =>
      el.addEventListener('click', (e) => {
        if (e.target.closest('[data-remove-layer]')) return;
        selectLayer(Number(el.dataset.idx));
      }));
    box.querySelectorAll('[data-remove-layer]').forEach((b) =>
      b.addEventListener('click', () => {
        if (state.busy) return;
        state.wm.layers.splice(Number(b.dataset.removeLayer), 1);
        if (state.wm.selLayer >= state.wm.layers.length) state.wm.selLayer = state.wm.layers.length - 1;
        renderWmLayers();
        syncEditorFromLayer();
        scheduleWmPreview();
        refreshButtons();
      }));
  }

  function selectLayer(i) {
    if (i < 0 || i >= state.wm.layers.length) return;
    if (state.wm.selLayer >= 0) readEditorToLayer();
    state.wm.selLayer = i;
    renderWmLayers();
    syncEditorFromLayer();
  }

  function syncEditorFromLayer() {
    const l = state.wm.layers[state.wm.selLayer];
    if (!l) {
      $('wm-layer-badge').textContent = '';
      return;
    }
    const isImage = l.kind === 'image';
    $('wm-row-text').classList.toggle('hidden', isImage);
    $('wm-row-font').classList.toggle('hidden', isImage);
    $('wm-row-image').classList.toggle('hidden', !isImage);
    if (!isImage) {
      $('wm-text').value = l.text || '';
      $('wm-font-size').value = l.fontSize;
      $('wm-color').value = l.color;
    } else {
      $('wm-image-path').textContent = l.imagePath || t('wm_no_image');
      $('wm-scale').value = l.imageScale;
    }
    $('wm-opacity').value = l.opacity;
    $('wm-opacity-val').textContent = l.opacity;
    $('wm-rotation').value = l.rotation;
    $('wm-position').value = l.position;
    $('wm-margin').value = l.margin;
    $('wm-gap').value = l.tileGap;
    $('wm-count').value = l.count;
    $('wm-layer-badge').textContent = t('wm_layer_label', { n: state.wm.selLayer + 1, total: state.wm.layers.length });
  }

  function readEditorToLayer() {
    const l = state.wm.layers[state.wm.selLayer];
    if (!l) return;
    if (l.kind === 'text') {
      l.text = $('wm-text').value;
      l.fontSize = parseInt($('wm-font-size').value, 10) || 48;
      l.color = $('wm-color').value;
    } else {
      l.imageScale = parseInt($('wm-scale').value, 10) || 40;
    }
    l.opacity = parseInt($('wm-opacity').value, 10) || 40;
    l.rotation = (parseInt($('wm-rotation').value, 10) || 0) % 360;
    l.position = $('wm-position').value;
    l.margin = parseInt($('wm-margin').value, 10) || 0;
    l.tileGap = parseInt($('wm-gap').value, 10) || 0;
    l.count = Math.min(Math.max(parseInt($('wm-count').value, 10) || 1, 1), 100);
  }

  function onWmEditorInput() {
    readEditorToLayer();
    renderWmLayers();
    scheduleWmPreview();
  }

  function wmLayers() {
    readEditorToLayer();
    return state.wm.layers.map((l) => ({
      kind: l.kind,
      text: l.text || '',
      fontSize: l.fontSize,
      color: l.color,
      opacity: l.opacity,
      rotation: l.rotation,
      position: l.position,
      margin: l.margin,
      tileGap: l.tileGap,
      imagePath: l.imagePath || null,
      imageScale: l.imageScale,
      count: l.count,
    }));
  }

  function renderWmList() {
    const box = $('wm-list');
    $('wm-count').textContent = t('count_files', { n: state.wm.files.length });
    if (state.wm.files.length === 0) {
      box.innerHTML = `<div class="empty">${esc(t('wm_empty_files'))}</div>`;
      return;
    }
    box.innerHTML = state.wm.files.map((f, i) => `
      <div class="file-item">
        <span class="fname" title="${esc(f.path)}">${esc(f.name)}</span>
        <span class="fmeta">${fmtSize(f.size)}</span>
        <button class="icon-btn" data-remove="${i}" title="${esc(t('btn_remove'))}">✕</button>
      </div>`).join('');
    box.querySelectorAll('[data-remove]').forEach((b) =>
      b.addEventListener('click', () => {
        if (state.busy) return;
        state.wm.files.splice(Number(b.dataset.remove), 1);
        renderWmList();
        refreshButtons();
      }));
  }

  async function wmAddFiles() {
    try {
      const filters = [[t('filter_images'), ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif', 'heic', 'heif']]];
      const picked = await call('pick_files', { filters }, t('status_picking_files'));
      if (!picked.length) return;
      const seen = new Set(state.wm.files.map((f) => f.path.toLowerCase()));
      for (const p of picked) {
        if (seen.has(p.toLowerCase())) continue;
        seen.add(p.toLowerCase());
        const meta = await metaOf(p);
        state.wm.files.push({ path: p, name: meta.name, size: meta.size });
      }
      state.wm.results = null;
      renderWmList();
      refreshButtons();
    } catch (e) { /* 已提示 */ }
  }

  const scheduleWmPreview = debounce(updateWmPreview, 300);

  async function updateWmPreview() {
    if (state.tool !== 'watermark') return;
    try {
      const layers = wmLayers();
      if (!layers.length) return;
      const url = await invoke('preview_watermark', { layers });
      state.wm.previewDataUrl = url;
      renderPreview();
    } catch (e) {
      /* 选项无效时静默，运行时给出提示 */
    }
  }

  async function wmPickImage() {
    try {
      const picked = await call('pick_files', {
        filters: [[t('filter_images'), ['png', 'jpg', 'jpeg', 'webp', 'bmp']]],
      }, t('status_picking_files'));
      if (!picked.length) return;
      const l = state.wm.layers[state.wm.selLayer];
      if (!l) return;
      l.imagePath = picked[0];
      $('wm-image-path').textContent = picked[0];
      renderWmLayers();
      scheduleWmPreview();
    } catch (e) { /* 已提示 */ }
  }

  async function wmRun() {
    if (state.busy) return;
    const files = state.wm.files.slice();
    if (!files.length) return;
    const layers = wmLayers();
    if (!layers.length) return;
    const jobId = 'job-' + Date.now();
    state.currentJob = jobId;
    const outDir = state.wm.outDir || null;
    setBusy(true, t('wm_status'));
    try {
      const results = await invoke('process_photo_watermarks', {
        jobId,
        paths: files.map((f) => f.path),
        layers,
        outDir,
      });
      state.wm.results = results;
      // 生成完成后清掉样式预览，只保留结果列表（避免残留水印画面）
      state.wm.previewDataUrl = null;
      const ok = results.filter((r) => r.ok).length;
      setStatus(t('wm_done', { ok, total: results.length }), results.length > ok);
      renderPreview();
    } catch (e) {
      state.wm.previewDataUrl = null;
      setStatus(t('op_failed', { msg: localizeError(e) }), true);
    } finally {
      setBusy(false);
      refreshButtons();
    }
  }

  function renderWmPreview(body) {
    const d = state.wm.previewDataUrl;
    let html = `<div class="pv-section-title">${esc(t('wm_preview_title'))}</div>`;
    if (d) {
      html += `<div class="wm-preview-box"><img src="${d}" alt="" /></div>
        <div class="pv-summary">${esc(t('wm_preview_hint'))}</div>`;
    } else {
      html += `<div class="empty wide">${esc(t('wm_preview_empty'))}</div>`;
    }
    if (state.wm.results) {
      const rs = state.wm.results;
      const ok = rs.filter((r) => r.ok);
      const dir = ok.length ? dirName(ok[0].outPath) : '';
      html += `<div class="pv-section-title">${esc(t('result_title'))}</div>
        <div class="pv-summary"><div>${esc(t('result_summary', { ok: ok.length, total: rs.length }))}</div></div>`;
      html += `<div class="pv-section-title">${esc(t('out_files_title'))}</div>`;
      html += rs.map((r) => `
        <div class="pv-row ${r.ok ? '' : 'error'}">
          <span class="new" title="${esc(r.ok ? r.outName : r.name)}">${esc(r.ok ? r.outName : r.name)}</span>
          ${r.ok ? '' : `<span class="flag warn">${esc(r.error ? localizeError(r.error) : t('err_failed'))}</span>`}
        </div>`).join('');
      if (dir) {
        html += `<div class="pv-summary"><button class="btn small" id="pv-open-wm">${esc(t('btn_open_out_dir'))}</button></div>`;
      }
    }
    body.innerHTML = html;
    if (state.wm.results) {
      const ok = state.wm.results.filter((r) => r.ok);
      if (ok.length) {
        body.querySelector('#pv-open-wm')?.addEventListener('click', () =>
          invoke('open_in_folder', { path: ok[0].outPath }).catch(() => {}));
      }
    }
  }

  // ============================================================
  // 右栏预览
  // ============================================================
  function renderPreview() {
    const body = $('preview-body');
    $('preview-tag').textContent = state.tool === 'rename' ? t('preview_tag_rename')
      : state.tool === 'images' ? t('preview_tag_images')
        : state.tool === 'watermark' ? t('wm_preview_tag') : t('preview_tag_pdf');
    if (state.tool === 'rename') renderRenamePreview(body);
    else if (state.tool === 'images') renderImagePreview(body);
    else if (state.tool === 'watermark') renderWmPreview(body);
    else renderPdfPreview(body);
  }

  function renderRenamePreview(body) {
    const entries = state.rename.preview;
    if (!entries.length) {
      body.innerHTML = `<div class="empty wide">${esc(t('preview_empty_rename'))}</div>`;
      return;
    }
    const conflicts = entries.filter((e) => e.conflict).length;
    const errors = entries.filter((e) => e.error).length;
    let html = `<div class="pv-section-title">${esc(t('preview_new_names', { n: entries.length }))}</div>`;
    html += entries.map((e) => {
      const errText = e.error ? localizeError(e.error) : '';
      return `
      <div class="pv-row ${e.error ? 'error' : e.conflict ? 'conflict' : ''}">
        <span class="old" title="${esc(e.oldName)}">${esc(e.oldName)}</span>
        <span class="arrow">→</span>
        <span class="new" title="${esc(errText || e.newName)}">${esc(errText || e.newName)}</span>
        ${e.conflict ? `<span class="flag warn">${esc(t('flag_conflict'))}</span>` : ''}
        ${e.error ? `<span class="flag bad">${esc(t('flag_invalid'))}</span>` : ''}
      </div>`;
    }).join('');
    if (conflicts || errors) {
      html += `<div class="pv-section-title">${esc(t('preview_conflict_summary', { n1: conflicts, n2: errors }))}</div>`;
    } else {
      html += `<div class="pv-section-title">${esc(t('preview_all_ok'))}</div>`;
    }
    body.innerHTML = html;
  }

  function renderImagePreview(body) {
    const sel = state.images.selected;
    const f = sel != null ? state.images.files[sel] : null;
    let html = '';
    if (f) {
      html += f.thumb
        ? `<img class="pv-big-img" src="${f.thumb}" alt="" />`
        : `<div class="empty wide">${esc(t('preview_no_thumb'))}</div>`;
      html += `<div class="pv-meta">
        <div><span class="k">${esc(t('meta_name'))}</span><span class="v">${esc(f.name)}</span></div>
        <div><span class="k">${esc(t('meta_size'))}</span><span class="v">${fmtSize(f.size)}</span></div>
        ${f.error
          ? `<div><span class="k">${esc(t('meta_status'))}</span><span class="v" style="color:var(--danger)">${esc(localizeError(f.error))}</span></div>`
          : `<div><span class="k">${esc(t('meta_dims'))}</span><span class="v">${f.width} × ${f.height}</span></div>
             <div><span class="k">${esc(t('meta_format'))}</span><span class="v">${esc(f.format)}（${esc(f.color)}）</span></div>`}
      </div>`;
    } else {
      html = `<div class="empty wide">${esc(t('preview_pick_card'))}</div>`;
    }
    if (state.images.results) {
      const rs = state.images.results;
      const ok = rs.filter((r) => r.ok);
      const inTotal = ok.reduce((s, r) => s + r.inSize, 0);
      const outTotal = ok.reduce((s, r) => s + (r.outSize || 0), 0);
      const saved = inTotal > 0 ? ((inTotal - outTotal) / inTotal * 100) : 0;
      const dir = ok.length ? dirName(ok[0].outPath) : '';
      html += `<div class="pv-section-title">${esc(t('result_title'))}</div>
        <div class="pv-summary">
          <div>${esc(t('result_summary', { ok: ok.length, total: rs.length }))}</div>
          ${inTotal ? `<div>${esc(t('result_size', { in: fmtSize(inTotal), out: fmtSize(outTotal), sign: saved >= 0 ? '−' : '+', pct: Math.abs(saved).toFixed(1) }))}</div>` : ''}
          <div><button class="btn small" id="pv-open-img-out">${esc(t('btn_open_out_dir'))}</button></div>
        </div>`;
      html += `<div class="pv-section-title">${esc(t('out_files_title'))}</div>`;
      html += rs.map((r) => `
        <div class="pv-row ${r.ok ? '' : 'error'}">
          <span class="new" title="${esc(r.ok ? r.outName : r.name)}">${esc(r.ok ? r.outName : r.name)}</span>
          ${r.ok
            ? `<span class="flag ok">${fmtSize(r.inSize)} → ${fmtSize(r.outSize)}</span>`
            : `<span class="flag warn">${esc(r.error ? localizeError(r.error) : t('err_failed'))}</span>`}
        </div>`).join('');
      body.innerHTML = html;
      if (dir) {
        body.querySelector('#pv-open-img-out')?.addEventListener('click', () =>
          invoke('open_in_folder', { path: ok[0].outPath }).catch(() => {}));
      }
      return;
    }
    body.innerHTML = html;
  }

  function renderPdfPreview(body) {
    if (state.pdf.mode === 'merge') return renderMergePreview(body);
    return renderSplitPreview(body);
  }

  function renderMergePreview(body) {
    const files = state.pdf.mergeFiles;
    let html = `<div class="pv-section-title">${esc(t('preview_merge_title', { n: files.length }))}</div>`;
    if (!files.length) {
      body.innerHTML = html + `<div class="empty wide">${esc(t('preview_merge_empty'))}</div>`;
      return;
    }
    const totalPages = files.reduce((s, f) => s + (f.pages || 0), 0);
    const totalSize = files.reduce((s, f) => s + (f.size || 0), 0);
    html += files.map((f, i) => `
      <div class="pv-pdf-item">
        <div class="n">${i + 1}. ${esc(f.name)}</div>
        <div class="m">${f.error ? esc(localizeError(f.error)) : t('pages_fmt', { n: f.pages || 0 }) + ' · ' + fmtSize(f.size) + (f.encrypted ? ' · ' + t('flag_encrypted') : '')}</div>
      </div>`).join('');
    html += `<div class="pv-section-title">${esc(t('merged_summary'))}</div>
      <div class="pv-summary">
        <div>${esc(t('merged_total', { n: totalPages, size: fmtSize(totalSize) }))}</div>
        <div><span class="k">${esc(t('out_to'))}</span><span class="v">${state.pdf.outPath ? esc(state.pdf.outPath) : esc(t('out_not_picked'))}</span></div>
      </div>`;
    if (state.pdf.results && state.pdf.results.type === 'merge') {
      const r = state.pdf.results;
      html += `<div class="pv-section-title">${esc(t('merge_done'))}</div>
        <div class="pv-summary">
          <div>${esc(t('merge_done_info', { n: r.pages, size: fmtSize(r.size) }))}</div>
          <div><button class="btn small" id="pv-open-merge">${esc(t('btn_reveal'))}</button></div>
        </div>`;
    }
    body.innerHTML = html;
    body.querySelector('#pv-open-merge')?.addEventListener('click', () =>
      invoke('open_in_folder', { path: state.pdf.results.outPath }).catch(() => {}));
  }

  function renderSplitPreview(body) {
    const s = state.pdf.source;
    if (!s) {
      body.innerHTML = `<div class="empty wide">${esc(t('empty_source'))}</div>`;
      return;
    }
    let html = `<div class="pv-section-title">${esc(t('split_source_title'))}</div>
      <div class="pv-pdf-item">
        <div class="n">${esc(s.name)}</div>
        <div class="m">${s.error ? esc(localizeError(s.error)) : t('pages_fmt', { n: s.pages }) + ' · ' + fmtSize(s.size)}</div>
      </div>`;
    if (!s.error) {
      html += `<div class="pv-section-title">${esc(t('split_plan_title'))}</div>`;
      if (state.pdf.partsError) {
        html += `<div class="pv-row error"><span class="new">${esc(localizeError(state.pdf.partsError))}</span></div>`;
      } else if (state.pdf.parts.length) {
        html += state.pdf.parts.map((p) => `
          <div class="pv-pdf-item">
            <div class="n">${esc(p.name)}</div>
            <div class="m">${esc(t('part_pages_range', { a: p.pages[0], b: p.pages[p.pages.length - 1], n: p.pages.length }))}</div>
          </div>`).join('');
        html += `<div class="pv-section-title">${esc(t('split_out_dir_title'))}</div>
          <div class="pv-summary"><span class="v">${esc(state.pdf.outDir || t('split_out_default'))}</span></div>`;
      }
    }
    if (state.pdf.results && state.pdf.results.type === 'split') {
      const rs = state.pdf.results.list;
      const ok = rs.filter((r) => r.ok);
      html += `<div class="pv-section-title">${esc(t('split_done_title', { ok: ok.length, total: rs.length }))}</div>`;
      html += rs.map((r) => `
        <div class="pv-row ${r.ok ? '' : 'error'}">
          <span class="new">${esc(r.name)}</span>
          ${r.ok
            ? `<span class="flag ok">${t('pages_fmt', { n: r.pages })} · ${fmtSize(r.size)}</span>`
            : `<span class="flag warn">${esc(r.error ? localizeError(r.error) : t('err_failed'))}</span>`}
        </div>`).join('');
      if (ok.length) {
        html += `<div class="pv-summary"><button class="btn small" id="pv-open-split">${esc(t('btn_open_out_dir'))}</button></div>`;
      }
    }
    body.innerHTML = html;
    body.querySelector('#pv-open-split')?.addEventListener('click', () => {
      const r = state.pdf.results.list.find((x) => x.ok);
      if (r) invoke('open_in_folder', { path: r.outPath }).catch(() => {});
    });
  }

  // ============================================================
  // 事件绑定
  // ============================================================
  function bindEvents() {
    // 重命名
    $('rename-add').addEventListener('click', renameAddFiles);
    $('rename-clear').addEventListener('click', () => {
      if (state.busy) return;
      state.rename.files = [];
      renderRenameList();
      updateRenamePreview();
      refreshButtons();
    });
    $('rename-apply').addEventListener('click', renameApply);
    ['r-template', 'r-find', 'r-replace', 'r-num-start', 'r-num-step'].forEach((id) =>
      $(id).addEventListener('input', scheduleRenamePreview));
    ['r-case'].forEach((id) =>
      $(id).addEventListener('change', scheduleRenamePreview));
    ['r-case-mode', 'r-space-mode', 'r-date-fmt'].forEach((id) =>
      $(id).addEventListener('change', updateRenamePreview));
    document.querySelectorAll('.token-chip').forEach((b) =>
      b.addEventListener('click', () => insertToken(b.dataset.token)));
    $('r-preset-num').addEventListener('click', () => applyPreset('{name}_{num:2}'));
    $('r-preset-date').addEventListener('click', () => applyPreset('{date}_{name}'));
    $('r-preset-upper').addEventListener('click', () => applyPreset('{upper}'));
    $('r-preset-orig').addEventListener('click', () => applyPreset('{name}'));

    // 图片
    $('img-add').addEventListener('click', imgAddFiles);
    $('img-clear').addEventListener('click', () => {
      if (state.busy) return;
      state.images.files = [];
      state.images.selected = null;
      state.images.results = null;
      renderImageList();
      renderPreview();
      refreshButtons();
    });
    $('img-run').addEventListener('click', imgRun);
    $('img-quality').addEventListener('input', () => {
      $('img-quality-val').textContent = $('img-quality').value;
    });
    $('img-format').addEventListener('change', () => {
      const f = $('img-format').value;
      $('img-quality').disabled = f === 'png';
      $('img-quality-hint').textContent = f === 'png' ? t('hint_quality_png')
        : f === 'keep' ? t('hint_quality_keep') : t('hint_quality_default');
    });
    $('img-resize-on').addEventListener('change', () => {
      $('img-resize-val').disabled = !$('img-resize-on').checked;
    });
    $('img-resize-mode').addEventListener('change', () => {
      $('img-resize-unit').textContent = $('img-resize-mode').value === 'percent' ? '%' : 'px';
    });
    document.querySelectorAll('input[name="img-outdir"]').forEach((r) =>
      r.addEventListener('change', () => {
        const custom = document.querySelector('input[name="img-outdir"]:checked').value === 'custom';
        $('img-outdir-pick').disabled = !custom;
        if (!custom) {
          state.images.outDir = '';
          $('img-outdir-path').textContent = '';
        }
      }));
    $('img-outdir-pick').addEventListener('click', async () => {
      try {
        const dir = await call('pick_folder', {}, t('status_picking_dir'));
        if (dir) {
          state.images.outDir = dir;
          $('img-outdir-path').textContent = dir;
        }
      } catch (e) { /* 已提示 */ }
    });

    // PDF
    $('pdf-mode-switch').addEventListener('click', (e) => {
      const btn = e.target.closest('button[data-mode]');
      if (btn) switchPdfMode(btn.dataset.mode);
    });
    $('pdf-add').addEventListener('click', pdfAddFiles);
    $('pdf-clear').addEventListener('click', () => {
      if (state.busy) return;
      state.pdf.mergeFiles = [];
      state.pdf.results = null;
      state.pdf.outPath = '';
      renderMergeList();
      renderPreview();
      refreshButtons();
    });
    $('pdf-out-name').addEventListener('input', () => {
      // 编辑文件名时保留用户已选定的输出目录
      if (!state.pdf.outPath) return;
      state.pdf.outPath = dirName(state.pdf.outPath) + '\\' + $('pdf-out-name').value;
      renderPreview();
    });
    $('pdf-out-path').addEventListener('click', pdfPickSave);
    $('pdf-merge-run').addEventListener('click', pdfMergeRun);
    $('pdf-src-pick').addEventListener('click', pdfPickSource);
    $('pdf-out-dir').addEventListener('click', pdfPickOutDir);
    $('pdf-split-run').addEventListener('click', pdfSplitRun);
    document.querySelectorAll('input[name="pdf-split-mode"]').forEach((r) =>
      r.addEventListener('change', () => {
        state.pdf.splitMode = document.querySelector('input[name="pdf-split-mode"]:checked').value;
        $('row-ranges').classList.toggle('hidden', state.pdf.splitMode !== 'ranges');
        $('row-every').classList.toggle('hidden', state.pdf.splitMode !== 'every');
        computeParts();
        renderPreview();
        refreshButtons();
      }));
    $('pdf-ranges').addEventListener('input', debounce(() => {
      computeParts();
      renderPreview();
      refreshButtons();
    }, 250));
    $('pdf-every').addEventListener('input', debounce(() => {
      computeParts();
      renderPreview();
      refreshButtons();
    }, 250));

    // 水印（仅图片）
    $('wm-add').addEventListener('click', wmAddFiles);
    $('wm-clear').addEventListener('click', () => {
      if (state.busy) return;
      state.wm.files = [];
      state.wm.results = null;
      state.wm.previewDataUrl = null;
      renderWmList();
      renderPreview();
      refreshButtons();
    });
    $('wm-run').addEventListener('click', wmRun);
    $('wm-add-text-layer').addEventListener('click', () => {
      if (state.busy) return;
      newLayer('text');
      refreshButtons();
      scheduleWmPreview();
    });
    $('wm-add-image-layer').addEventListener('click', () => {
      if (state.busy) return;
      newLayer('image');
      refreshButtons();
      scheduleWmPreview();
    });
    $('wm-image-pick').addEventListener('click', wmPickImage);
    ['wm-text', 'wm-font-size', 'wm-rotation', 'wm-margin', 'wm-gap', 'wm-scale',
      'wm-count', 'wm-position', 'wm-color', 'wm-opacity'].forEach((id) =>
      $(id).addEventListener('input', onWmEditorInput));
    $('wm-opacity').addEventListener('input', () => {
      $('wm-opacity-val').textContent = $('wm-opacity').value;
    });
    document.querySelectorAll('input[name="wm-outdir"]').forEach((r) =>
      r.addEventListener('change', () => {
        const custom = document.querySelector('input[name="wm-outdir"]:checked').value === 'custom';
        $('wm-outdir-pick').disabled = !custom;
        if (!custom) {
          state.wm.outDir = '';
          $('wm-outdir-path').textContent = '';
        }
      }));
    $('wm-outdir-pick').addEventListener('click', async () => {
      try {
        const dir = await call('pick_folder', {}, t('status_picking_dir'));
        if (dir) {
          state.wm.outDir = dir;
          $('wm-outdir-path').textContent = dir;
        }
      } catch (e) { /* 已提示 */ }
    });

    // 关于对话框
    function openAbout() {
      renderAboutDeps();
      $('about-overlay').classList.remove('hidden');
    }
    function closeAbout() {
      $('about-overlay').classList.add('hidden');
    }
    $('about-open').addEventListener('click', openAbout);
    $('about-close').addEventListener('click', closeAbout);
    $('about-ok').addEventListener('click', closeAbout);
    $('about-overlay').addEventListener('click', (e) => {
      if (e.target === $('about-overlay')) closeAbout();
    });
    $('about-email').addEventListener('click', (e) => {
      e.preventDefault();
      invoke('open_url', { url: 'mailto:d1637_fzza6naycz@aka.yeah.net' }).catch(() => {});
    });
    $('about-repo').addEventListener('click', (e) => {
      e.preventDefault();
      invoke('open_url', { url: 'https://github.com/Willuconrl/offline-file-tool' }).catch(() => {});
    });

    // 语言切换
    $('lang-select').addEventListener('change', (e) => {
      if (state.busy) {
        setStatus(t('busy_msg'), true);
        $('lang-select').value = lang;
        return;
      }
      setLang(e.target.value);
    });
  }

  // 第三方依赖清单（名称与许可证固定，用途随语言）
  function renderAboutDeps() {
    const deps = [
      { n: 'Tauri', l: 'MIT / Apache-2.0', p: t('dep_tauri') },
      { n: 'tauri-plugin-dialog', l: 'MIT / Apache-2.0', p: t('dep_tauri') },
      { n: 'image', l: 'MIT / Apache-2.0', p: t('dep_image') },
      { n: 'libwebp (webp)', l: 'BSD-3-Clause', p: t('dep_webp') },
      { n: 'libheif / libde265', l: 'LGPL-3.0', p: t('dep_heif') },
      { n: 'lopdf', l: 'MIT', p: t('dep_lopdf') },
      { n: 'ab_glyph', l: 'Apache-2.0', p: t('dep_abglyph') },
      { n: 'serde / serde_json', l: 'MIT / Apache-2.0', p: t('dep_serde') },
      { n: 'base64', l: 'MIT / Apache-2.0', p: t('dep_serde') },
    ];
    $('about-deps').innerHTML = deps.map((d) => `
      <div class="dep-item">
        <span class="dn">${esc(d.n)}</span><span class="dl">${esc(d.l)}</span>
        <div class="dp">${esc(d.p)}</div>
      </div>`).join('');
  }

  // ---------- 初始化 ----------
  // 禁用浏览器式右键菜单（返回/刷新/另存为）
  document.addEventListener('contextmenu', (e) => e.preventDefault());
  // 禁用输入框自动填充/历史值下拉（Tauri 本地应用内无意义且易误触）
  document.querySelectorAll('input, textarea, select').forEach((el) => {
    el.setAttribute('autocomplete', 'off');
    el.setAttribute('spellcheck', 'false');
  });
  $('lang-select').value = lang;
  applyI18n();
  listen('job', (event) => onJobProgress(event.payload));
  bindEvents();
  renderRenameList();
  renderImageList();
  renderMergeList();
  renderSplitSource();
  renderWmList();
  if (!state.wm.layers.length) {
    newLayer('text');
  } else {
    state.wm.selLayer = 0;
    renderWmLayers();
    syncEditorFromLayer();
  }
  renderPreview();
  refreshButtons();
  setStatus(t('status_ready'));
})();
