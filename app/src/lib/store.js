import { writable } from 'svelte/store'

export const connected = writable(false)
export const telemetry = writable(null)
export const programs = writable([])
export const toolpath = writable(null)
export const loadedInfo = writable(null)
export const rejection = writable(null)
/// 'library' | 'digitize' | null — overlay drawers are exclusive
export const activePanel = writable(null)

export const scans = writable([])
/// points currently shown in the viewport: { name, points: [[x,y,z],…], mode }
export const scanCloud = writable(null)
export const tools = writable([])

let socket = null
let retryTimer = null
let rejectionTimer = null

export function connect() {
  const url = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`
  socket = new WebSocket(url)
  socket.onopen = () => connected.set(true)
  socket.onclose = () => {
    connected.set(false)
    clearTimeout(retryTimer)
    retryTimer = setTimeout(connect, 1000)
  }
  socket.onmessage = (e) => {
    const msg = JSON.parse(e.data)
    if (msg.event === 'rejected') {
      showRejection(msg.cmd, msg.reason)
    } else {
      telemetry.set(msg)
    }
  }
}

function showRejection(cmd, reason) {
  rejection.set({ cmd, reason })
  clearTimeout(rejectionTimer)
  rejectionTimer = setTimeout(() => rejection.set(null), 4000)
}

export function send(cmd) {
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(cmd))
}

export async function refreshPrograms() {
  const res = await fetch('/api/programs')
  programs.set(await res.json())
}

export async function loadProgram(name) {
  const res = await fetch(`/api/programs/${encodeURIComponent(name)}/load`, { method: 'POST' })
  if (!res.ok) {
    const body = await res.json().catch(() => ({}))
    showRejection('load', body.detail || res.statusText)
    return
  }
  loadedInfo.set(await res.json())
  const tp = await (await fetch('/api/toolpath')).json()
  toolpath.set(tp)
  activePanel.set(null)
}

export async function uploadProgram(file) {
  const form = new FormData()
  form.append('file', file)
  const res = await fetch('/api/programs', { method: 'POST', body: form })
  if (!res.ok) {
    const body = await res.json().catch(() => ({}))
    showRejection('upload', body.detail || res.statusText)
    return
  }
  await refreshPrograms()
}

// ── tool table ──────────────────────────────────────────────────────────

export async function refreshTools() {
  const res = await fetch('/api/tools')
  if (res.ok) tools.set(await res.json())
}

export async function saveTools(table) {
  const res = await fetch('/api/tools', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(table),
  })
  if (!res.ok) {
    const body = await res.json().catch(() => ({}))
    showRejection('tools', body.detail || res.statusText)
    return
  }
  await refreshTools()
}

// ── digitizing ──────────────────────────────────────────────────────────

async function scanCall(method, path, body) {
  const res = await fetch(`/api/scans${path}`, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  })
  if (!res.ok) {
    const detail = (await res.json().catch(() => ({}))).detail || res.statusText
    showRejection('scan', detail)
    return null
  }
  return res.json()
}

export const refreshScans = async () => scans.set((await scanCall('GET', '')) ?? [])
export const createScan = (name) => scanCall('POST', '', { name })
export const capturePoint = () => scanCall('POST', '/active/point')
export const undoPoint = () => scanCall('POST', '/active/undo')
export const finishScan = () => scanCall('POST', '/active/finish')
export const discardScan = () => scanCall('POST', '/active/discard')
export const startGridScan = (params) => scanCall('POST', '/active/grid', params)
export const cancelGridScan = () => scanCall('POST', '/active/cancel')
export const deleteScan = (name) => scanCall('DELETE', `/${encodeURIComponent(name)}`)
export const exportScan = (name, params) => scanCall('POST', `/${encodeURIComponent(name)}/export`, params)

export async function showScanCloud(name) {
  const data = await scanCall('GET', name === null ? '/active' : `/${encodeURIComponent(name)}`)
  scanCloud.set(data?.points?.length ? data : null)
}
