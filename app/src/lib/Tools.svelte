<script>
  import { onMount } from 'svelte'
  import { tools, activePanel, refreshTools, saveTools } from './store.js'

  let edits = $state([])
  let dirty = $state(false)

  onMount(async () => {
    await refreshTools()
    edits = $tools.map((t) => ({ ...t }))
  })

  function addTool() {
    const next = edits.length ? Math.max(...edits.map((t) => +t.number || 0)) + 1 : 1
    edits.push({ number: next, diameter: 6, length: 0, note: '' })
    dirty = true
  }

  function removeTool(i) {
    edits.splice(i, 1)
    dirty = true
  }

  async function save() {
    const clean = edits
      .map((t) => ({ number: +t.number, diameter: +t.diameter || 0, length: +t.length || 0, note: t.note ?? '' }))
      .filter((t) => t.number > 0)
    await saveTools(clean)
    edits = $tools.map((t) => ({ ...t }))
    dirty = false
  }
</script>

<div class="drawer panel">
  <div class="head">
    <span class="label">Tool Table</span>
    <button class="close" onclick={() => activePanel.set(null)}>✕</button>
  </div>

  <div class="cols label-row">
    <span>#</span><span>Ø mm</span><span>Length</span><span>Note</span><span></span>
  </div>

  <div class="list">
    {#each edits as t, i}
      <div class="cols">
        <input type="number" min="1" bind:value={t.number} oninput={() => (dirty = true)} />
        <input type="number" step="0.1" bind:value={t.diameter} oninput={() => (dirty = true)} />
        <input type="number" step="0.01" bind:value={t.length} oninput={() => (dirty = true)} />
        <input type="text" bind:value={t.note} oninput={() => (dirty = true)} />
        <button class="del" onclick={() => removeTool(i)}>✕</button>
      </div>
    {:else}
      <span class="empty">Tool table is empty</span>
    {/each}
  </div>

  <div class="row">
    <button class="btn" onclick={addTool}>+ Add tool</button>
    <button class="btn primary" disabled={!dirty} onclick={save}>Save table</button>
  </div>

  <p class="hint">
    Length is the tool's Z offset, applied by G43 in programs and by manual
    tool selection. Touch off Z with the active tool to combine both.
  </p>
</div>

<style>
  .drawer {
    position: absolute;
    top: 14px;
    left: 14px;
    bottom: 14px;
    width: 360px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    background: rgba(13, 15, 18, 0.88);
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
    animation: slide-in 220ms var(--ease);
    z-index: 10;
    overflow-y: auto;
  }

  @keyframes slide-in {
    from {
      transform: translateX(-16px);
      opacity: 0;
    }
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .close {
    color: var(--text-faint);
    font-size: 14px;
    padding: 4px 8px;
    border-radius: var(--r-sm);
  }

  .close:hover {
    color: var(--text);
  }

  .cols {
    display: grid;
    grid-template-columns: 48px 64px 70px 1fr 26px;
    gap: 6px;
    align-items: center;
  }

  .label-row span {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .cols input {
    width: 100%;
    padding: 7px 8px;
    border-radius: var(--r-sm);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
    font-family: var(--font-mono);
    font-size: 12px;
    outline: none;
  }

  .cols input:focus {
    border-color: rgba(39, 224, 255, 0.5);
  }

  .del {
    color: var(--text-faint);
    font-size: 12px;
    border-radius: var(--r-sm);
    height: 26px;
  }

  .del:hover {
    color: var(--danger);
    background: var(--danger-dim);
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .row .btn {
    flex: 1;
  }

  .empty {
    color: var(--text-faint);
    font-size: 13px;
    padding: 8px;
  }

  .hint {
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-faint);
  }
</style>
