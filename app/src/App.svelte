<script>
  import StatusBar from './lib/StatusBar.svelte'
  import Viewport from './lib/Viewport.svelte'
  import DRO from './lib/DRO.svelte'
  import Jog from './lib/Jog.svelte'
  import Overrides from './lib/Overrides.svelte'
  import Transport from './lib/Transport.svelte'
  import Library from './lib/Library.svelte'
  import Digitize from './lib/Digitize.svelte'
  import Tools from './lib/Tools.svelte'
  import Code from './lib/Code.svelte'
  import Spindle from './lib/Spindle.svelte'
  import Toast from './lib/Toast.svelte'
  import { activePanel, refreshTools, telemetry, send } from './lib/store.js'
  import { onMount } from 'svelte'

  refreshTools()

  // keyboard pendant: arrows X/Y · PgUp/PgDn Z · , . A · space hold · esc stop
  const JOG_KEYS = {
    ArrowLeft: ['x', -1],
    ArrowRight: ['x', 1],
    ArrowUp: ['y', 1],
    ArrowDown: ['y', -1],
    PageUp: ['z', 1],
    PageDown: ['z', -1],
    ',': ['a', -1],
    '.': ['a', 1],
  }
  const held = new Set()

  function typing(e) {
    return ['INPUT', 'TEXTAREA', 'SELECT'].includes(e.target?.tagName)
  }

  onMount(() => {
    const down = (e) => {
      if (typing(e)) return
      const state = $telemetry?.state
      if (e.key === ' ') {
        e.preventDefault()
        if (state === 'running') send({ cmd: 'pause' })
        else if (state === 'paused') send({ cmd: 'resume' })
        return
      }
      if (e.key === 'Escape') {
        send({ cmd: 'stop' })
        return
      }
      const jog = JOG_KEYS[e.key]
      if (!jog || e.repeat) return
      e.preventDefault()
      held.add(e.key)
      send({ cmd: 'jog', axis: jog[0], dir: jog[1], velocity: 3000 })
    }
    const up = (e) => {
      const jog = JOG_KEYS[e.key]
      if (!jog || !held.delete(e.key)) return
      send({ cmd: 'jog', axis: jog[0], dir: 0 })
    }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
    }
  })
</script>

<div class="shell">
  <StatusBar />
  <main>
    <section class="stage">
      <Viewport />
      {#if $activePanel === 'library'}
        <Library />
      {:else if $activePanel === 'digitize'}
        <Digitize />
      {:else if $activePanel === 'tools'}
        <Tools />
      {:else if $activePanel === 'code'}
        <Code />
      {/if}
      <Toast />
    </section>
    <aside>
      <DRO />
      <Jog />
      <Spindle />
      <Overrides />
    </aside>
  </main>
  <Transport />
</div>

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px;
  }

  main {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 10px;
  }

  .stage {
    position: relative;
    flex: 1;
    min-width: 0;
    border-radius: var(--r-lg);
    overflow: hidden;
    border: 1px solid var(--hairline);
    background: radial-gradient(1200px 800px at 30% 20%, #0a0d11 0%, #000 70%);
  }

  aside {
    width: 320px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
  }
</style>
