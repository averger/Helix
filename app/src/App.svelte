<script>
  import StatusBar from './lib/StatusBar.svelte'
  import Viewport from './lib/Viewport.svelte'
  import DRO from './lib/DRO.svelte'
  import Jog from './lib/Jog.svelte'
  import Overrides from './lib/Overrides.svelte'
  import Transport from './lib/Transport.svelte'
  import Library from './lib/Library.svelte'
  import Digitize from './lib/Digitize.svelte'
  import Toast from './lib/Toast.svelte'
  import { activePanel } from './lib/store.js'
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
      {/if}
      <Toast />
    </section>
    <aside>
      <DRO />
      <Jog />
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
