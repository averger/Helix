import { writable } from 'svelte/store'

export const connected = writable(false)
export const telemetry = writable(null)
export const programs = writable([])
export const toolpath = writable(null)
export const loadedInfo = writable(null)
export const rejection = writable(null)
export const libraryOpen = writable(false)

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
  libraryOpen.set(false)
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
