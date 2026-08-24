import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open } from '@tauri-apps/plugin-dialog'
import { openPath } from '@tauri-apps/plugin-opener'

const settings = { video: { encoder: 'gpu', quality: 'high' }, image: { quality: 'high', img_format: 'same' } }
const encLabel = {
  x265: 'H.265', x264: 'H.264',
  nvenc: '显卡·NVENC', qsv: '显卡·QSV', amf: '显卡·AMF', videotoolbox: '显卡·VideoToolbox',
  gpu: '显卡加速',
}
const vQualLabel = { lossless: '视觉无损', high: '高画质', compact: '高压缩' }
const iQualLabel = { high: '高画质', balanced: '均衡', compact: '高压缩' }
const kindLabel = { video: '🎬', image: '🖼️' }
let OUTPUT_DIR = ''
let curTab = 'video'

function bind(id, key, grp) {
  document.querySelectorAll('#' + id + ' .opt').forEach(btn => {
    btn.onclick = () => {
      document.querySelectorAll('#' + id + ' .opt').forEach(b => b.classList.remove('active'))
      btn.classList.add('active')
      settings[grp][key] = btn.dataset.v
    }
  })
}
bind('encOpt', 'encoder', 'video')
bind('qOpt', 'quality', 'video')
bind('iqOpt', 'quality', 'image')
bind('ifOpt', 'img_format', 'image')

document.querySelectorAll('.tab').forEach(t => {
  t.onclick = () => {
    curTab = t.dataset.tab
    document.querySelectorAll('.tab').forEach(x => x.classList.toggle('active', x === t))
    document.querySelectorAll('.panel').forEach(p => p.classList.toggle('active', p.id === 'panel-' + curTab))
  }
})

const queue = document.getElementById('queue')

function fmt(b) {
  if (b == null || b === 0) return '—'
  const u = ['B', 'KB', 'MB', 'GB']
  let i = 0
  while (b >= 1024 && i < 3) { b /= 1024; i++ }
  return b.toFixed(i ? 1 : 0) + ' ' + u[i]
}

function outPathOf(j) {
  if (j.out_path) return j.out_path
  if (OUTPUT_DIR && j.out_name) return OUTPUT_DIR + '/' + j.out_name
  return ''
}

function itemHTML(j) {
  const chip = { running: '压缩中', done: '已完成', error: '失败' }[j.status] || j.status
  let meta = j.meta || ''
  let fillCls = 'fill'
  if (j.status === 'done') fillCls = 'fill done'
  else if (j.status === 'error') fillCls = 'fill error'
  const kind = j.kind || 'video'
  const delBtn = (j.status === 'done' || j.status === 'error')
    ? "<button class='del' title='删除记录' data-del='" + j.id + "'>×</button>" : ''

  // 进度条宽度：完成 100%，错误 0%，进行中取实时进度
  const widthPct = j.status === 'done' ? 100
                  : j.status === 'error' ? 0
                  : (j.progress || 0)

  let actions = ''
  if (j.status === 'done') {
    const pct = j.src_size ? ((1 - j.out_size / j.src_size) * 100).toFixed(1) : 0
    const ql = kind === 'image' ? (iQualLabel[j.quality] || '') : (j.encoder ? (encLabel[j.encoder] || j.encoder) : '')
    let extra = ''
    if (kind === 'image' && j.img_format === 'webp') extra = ' · WebP'
    meta = fmt(j.src_size) + ' → ' + fmt(j.out_size) + " <span class='save'>↓ " + pct + '%</span> · ' + ql + extra
    const p = outPathOf(j)
    actions = "<div class='btns'>" +
              "<button class='btn' data-open='" + (p || '') + "'>打开文件</button>" +
              "<button class='btn ghost' data-folder='1'>在文件夹中显示</button>" +
              delBtn +
              "</div>"
  } else if (j.status === 'error') {
    meta = j.error || '处理失败'
    actions = "<div class='btns'>" + delBtn + "</div>"
  } else {
    meta = '压缩中 ' + (j.progress || 0) + '%' + (j.speed ? ' · ' + j.speed : '')
  }

  return "<div class='row1'><span class='name'>" + kindLabel[kind] + ' ' + (j.name || '') + "</span><span class='chip " + j.status + "'>" + chip + "</span></div>" +
    "<div class='bar'><div class='" + fillCls + "' style='width:" + widthPct + "%'></div></div>" +
    "<div class='meta'>" + meta + "</div>" + actions
}

function updateItem(j) {
  let el = document.getElementById(j.id)
  if (!el) {
    el = document.createElement('div')
    el.className = 'item'
    el.id = j.id
    queue.insertBefore(el, queue.firstChild)
  }
  el.innerHTML = itemHTML(j)
}

listen('progress', (e) => updateItem(e.payload))

async function pickFiles(kind) {
  const filters = kind === 'video'
    ? [{ name: '视频', extensions: ['mp4', 'mov', 'mkv', 'avi', 'flv', 'wmv', 'ts', 'm4v'] }]
    : [{ name: '图片', extensions: ['jpg', 'jpeg', 'png', 'webp', 'bmp', 'tif', 'tiff'] }]
  const selected = await open({ multiple: true, filters })
  if (!selected) return
  const paths = Array.isArray(selected) ? selected : [selected]
  paths.forEach(p => addPath(p, kind))
}

async function addPath(path, kind) {
  const name = path.split(/[\\/]/).pop()
  const id = 'j_' + Math.random().toString(36).slice(2)
  const el = document.createElement('div')
  el.className = 'item'
  el.id = id
  el.innerHTML = itemHTML({ kind, name, status: 'running', progress: 0, meta: '准备中…' })
  queue.insertBefore(el, queue.firstChild)
  const s = settings[kind]
  try {
    const res = await invoke('compress', {
      id, path, kind,
      // Rust 端要求所有 key 必填；图片没有 encoder、视频没有 img_format，
      // undefined 会被 JSON 丢弃导致 IPC 报 missing key，故都补默认值（Rust 按类型忽略）
      encoder: s.encoder || '',
      quality: s.quality || 'high',
      imgFormat: s.img_format || 'same',
    })
    updateItem(res)
  } catch (e) {
    updateItem({ id, kind, name, status: 'error', error: String(e) })
  }
}

document.getElementById('dropVideo').onclick = () => pickFiles('video')
document.getElementById('dropImage').onclick = () => pickFiles('image')

getCurrentWebview().onDragDropEvent((event) => {
  if (event.payload.type === 'drop') {
    event.payload.paths.forEach(p => addPath(p, curTab))
  }
})

queue.addEventListener('click', async (e) => {
  const del = e.target.closest('[data-del]')
  if (del) {
    const id = del.dataset.del
    await invoke('delete_record', { id })
    const el = document.getElementById(id)
    if (el) el.remove()
    return
  }
  const openBtn = e.target.closest('[data-open]')
  if (openBtn) {
    const p = openBtn.dataset.open
    if (p) { try { await openPath(p) } catch (err) { alert('打开失败：' + err) } }
    return
  }
  const folderBtn = e.target.closest('[data-folder]')
  if (folderBtn && OUTPUT_DIR) {
    try { await openPath(OUTPUT_DIR) } catch (err) { alert('打开失败：' + err) }
  }
})

document.getElementById('clearAll').onclick = async () => {
  if (!confirm('确定清空所有压缩记录及对应的输出文件？')) return
  await invoke('clear_records')
  queue.innerHTML = ''
}

async function loadRecords() {
  try {
    const recs = await invoke('get_records')
    recs.forEach(updateItem)
  } catch (e) { /* 忽略 */ }
}

;(async () => {
  try {
    const env = await invoke('env_info')
    if (!env.ffmpeg) document.getElementById('envWarn').hidden = false
    if (env.output_dir) OUTPUT_DIR = env.output_dir
    if (!env.webp) {
      const wb = document.querySelector("#ifOpt .opt[data-v='webp']")
      if (wb) wb.style.display = 'none'
      settings.image.img_format = 'same'
    }
    // 无 GPU 编码器时隐藏「显卡加速」并回退到 H.265
    const gpus = env.gpu_encoders || []
    const gpuBtn = document.querySelector("#encOpt .opt[data-v='gpu']")
    if (!gpus.length && gpuBtn) {
      gpuBtn.style.display = 'none'
      if (settings.video.encoder === 'gpu') settings.video.encoder = 'x265'
      document.querySelectorAll('#encOpt .opt').forEach(b => b.classList.toggle('active', b.dataset.v === settings.video.encoder))
    }
  } catch (e) { /* 忽略 */ }
  loadRecords()
})()
