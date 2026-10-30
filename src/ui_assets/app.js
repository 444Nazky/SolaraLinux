const $ = s => document.querySelector(s);
const rows = $('#rows');
const logEl = $('#log');

let selected = null;
const attached = new Set();

function esc(s) {
  return String(s).replace(/[&<>"']/g, c => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[c]));
}

function log(text, kind = '') {
  const atBottom = logEl.scrollTop + logEl.clientHeight >= logEl.scrollHeight - 30;
  const span = document.createElement('span');
  span.className = kind ? 'l-' + kind : '';
  span.textContent = text.endsWith('\n') ? text : text + '\n';
  logEl.appendChild(span);
  if (atBottom) logEl.scrollTop = logEl.scrollHeight;
  while (logEl.childNodes.length > 2000) logEl.removeChild(logEl.firstChild);
}

function clearLog() {
  logEl.textContent = '';
}

function status(s, live) {
  $('#status').textContent = s;
  $('#dot').className = 'dot' + (live ? ' on' : '');
}

async function api(path, data) {
  const opt = data
    ? {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams(data),
      }
    : { method: 'GET' };
  const res = await fetch(path, opt);
  if (!res.ok) throw new Error('HTTP ' + res.status);
  return await res.text();
}

function select(pid) {
  selected = pid;
  $('#target').textContent = pid ? 'pid ' + pid : 'no target';
  $('#target').className = 'chip' + (pid ? ' live' : '');
  $('#btnExec').disabled = !pid;
  $('#btnRead').disabled = !pid;
  $('#btnScan').disabled = !pid;
  [...rows.querySelectorAll('tr')].forEach(tr =>
    tr.classList.toggle('sel', tr.dataset.pid === String(pid)));
}

function render(list) {
  if (!list.length) {
    rows.innerHTML =
      '<tr><td colspan="5" class="empty">no roblox / sober / wine process found' +
      '<br><b>launch the client first, then refresh</b></td></tr>';
    $('#count').textContent = '0';
    return;
  }
  rows.innerHTML = '';
  for (const line of list) {
    const [pid, fp, ns, ...rest] = line.split('\t');
    const cmd = rest.join('\t');
    const base = cmd.split(' ')[0].split('/').pop() || cmd;
    const tr = document.createElement('tr');
    tr.dataset.pid = pid;
    tr.onclick = e => {
      if (e.target.tagName !== 'BUTTON') select(Number(pid));
    };
    const isAtt = attached.has(String(pid));
    tr.innerHTML =
      `<td class="pid">${esc(pid)}</td>` +
      `<td><div class="name">${esc(base)}</div>` +
      `<div class="cmd" title="${esc(cmd)}">${esc(cmd)}</div></td>` +
      `<td><span class="tag ${ns === 'isolated' ? 'iso' : ''}">${esc(ns)}</span></td>` +
      `<td><span class="tag">${esc(fp)}</span></td>` +
      `<td style="text-align:right">` +
      (isAtt
        ? '<span class="tag att">attached</span>'
        : `<button class="sm" onclick="attach(${Number(pid)}, this)">Attach</button>`) +
      '</td>';
    rows.appendChild(tr);
  }
  $('#count').textContent = list.length;
  if (selected && !list.some(l => l.split('\t')[0] === String(selected))) select(null);
}

async function refresh() {
  status('scanning', false);
  try {
    const text = await api('/api/processes');
    const list = text.trim() ? text.trim().split('\n') : [];
    render(list);
    status(list.length ? list.length + ' target(s)' : 'idle', list.length > 0);
  } catch (e) {
    log('error  refresh: ' + e.message, 'err');
    status('error', false);
  }
}

async function attach(pid, btn) {
  btn.disabled = true;
  btn.textContent = '…';
  status('attaching', false);
  log('→ attach pid ' + pid, 'in');
  try {
    const out = await api('/api/attach', { pid });
    for (const line of out.split('\n')) {
      if (!line.trim()) continue;
      const kind = line.startsWith('error') ? 'err'
        : line.startsWith('ok') ? 'ok' : 'dim';
      log(line, kind);
    }
    if (out.includes('ok     attached')) {
      attached.add(String(pid));
      select(pid);
      status('attached ' + pid, true);
    } else {
      status('attach failed', false);
      btn.disabled = false;
      btn.textContent = 'Attach';
    }
  } catch (e) {
    log('error  ' + e.message, 'err');
    status('error', false);
    btn.disabled = false;
    btn.textContent = 'Attach';
  }
}

async function run() {
  if (!selected) return;
  const script = $('#script').value;
  log('→ exec pid ' + selected + ' (' + script.length + 'b)', 'in');
  try {
    log((await api('/api/exec', { pid: selected, script })).trimEnd(), 'ok');
    status('executed', true);
  } catch (e) {
    log('error  ' + e.message, 'err');
    status('error', false);
  }
}

async function readMem() {
  if (!selected) return;
  const addr = $('#addr').value.trim();
  const len = $('#len').value.trim() || '64';
  log('→ read ' + addr + ' +' + len, 'in');
  try {
    log((await api('/api/read', { pid: selected, addr, len })).trimEnd());
  } catch (e) {
    log('error  ' + e.message, 'err');
  }
}

async function scanMem() {
  if (!selected) return;
  const pattern = $('#pat').value.trim();
  if (!pattern) return;
  log('→ scan ' + pattern, 'in');
  try {
    log((await api('/api/scan', { pid: selected, pattern })).trimEnd());
  } catch (e) {
    log('error  ' + e.message, 'err');
  }
}

function toggleHint() {
  const h = $('#hint');
  h.style.display = h.style.display === 'none' ? 'block' : 'none';
}

document.addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
    e.preventDefault();
    run();
  }
});

log('solara ui ready', 'dim');
refresh();
setInterval(refresh, 5000);
