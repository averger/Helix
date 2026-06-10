<script>
  import { onMount } from 'svelte'
  import { programs, loadedInfo, libraryOpen, refreshPrograms, loadProgram, uploadProgram } from './store.js'

  let fileInput = $state(null)

  onMount(refreshPrograms)

  function fmtSize(bytes) {
    return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} kB`
  }

  async function onPick(e) {
    const file = e.currentTarget.files?.[0]
    if (file) await uploadProgram(file)
    e.currentTarget.value = ''
  }
</script>

<div class="drawer panel">
  <div class="head">
    <span class="label">Program Library</span>
    <button class="close" onclick={() => libraryOpen.set(false)}>✕</button>
  </div>

  <div class="list">
    {#each $programs as p (p.name)}
      <button class="item" class:active={$loadedInfo?.name === p.name} onclick={() => loadProgram(p.name)}>
        <span class="pname">{p.name}</span>
        <span class="size">{fmtSize(p.size)}</span>
      </button>
    {:else}
      <span class="empty">Library is empty</span>
    {/each}
  </div>

  <button class="btn" onclick={() => fileInput.click()}>↑ Upload G-code</button>
  <input bind:this={fileInput} type="file" accept=".ngc,.nc,.gcode" hidden onchange={onPick} />
</div>

<style>
  .drawer {
    position: absolute;
    top: 14px;
    left: 14px;
    bottom: 14px;
    width: 300px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    background: rgba(13, 15, 18, 0.88);
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
    animation: slide-in 220ms var(--ease);
    z-index: 10;
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

  .list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .item {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    border: 1px solid var(--hairline);
    background: var(--panel-2);
    text-align: left;
    transition: border-color 150ms var(--ease), background 150ms var(--ease);
  }

  .item:hover {
    border-color: var(--hairline-2);
    background: #181c22;
  }

  .item.active {
    border-color: rgba(39, 224, 255, 0.5);
    background: var(--accent-dim);
  }

  .pname {
    font-family: var(--font-mono);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    font-size: 11px;
    color: var(--text-faint);
    white-space: nowrap;
  }

  .empty {
    color: var(--text-faint);
    font-size: 13px;
    padding: 12px;
  }
</style>
