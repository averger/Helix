<script>
  import { connected, telemetry, send } from './store.js'

  const STATE_META = {
    estop: { label: 'E-STOP', tone: 'danger' },
    off: { label: 'POWER OFF', tone: 'dim' },
    on: { label: 'UNHOMED', tone: 'warn' },
    homing: { label: 'HOMING', tone: 'warn' },
    idle: { label: 'READY', tone: 'ok' },
    jog: { label: 'JOG', tone: 'live' },
    mdi: { label: 'MDI', tone: 'live' },
    running: { label: 'RUNNING', tone: 'live' },
    paused: { label: 'PAUSED', tone: 'warn' },
    probing: { label: 'PROBING', tone: 'live' },
  }

  let state = $derived($telemetry?.state ?? 'estop')
  let meta = $derived(STATE_META[state] ?? STATE_META.estop)
  let alarms = $derived($telemetry?.alarms ?? [])
</script>

<header>
  <div class="brand">
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M5 3c6 2 8 4 8 6s-3 3-3 6 2 4 9 6M19 3c-6 2-8 4-8 6s3 3 3 6-2 4-9 6"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
      />
    </svg>
    <span class="wordmark">HELIX</span>
  </div>

  <div class="state {meta.tone}">
    <span class="dot"></span>
    {meta.label}
  </div>

  {#if alarms.length}
    <div class="alarm" title={alarms.join('\n')}>{alarms[alarms.length - 1]}</div>
  {/if}

  <div class="spacer"></div>

  <div class="conn" class:online={$connected}>
    {$connected ? 'LINK' : 'NO LINK'}
  </div>

  {#if state === 'estop'}
    <button class="btn ghost-accent" onclick={() => send({ cmd: 'estop_reset' })}>Reset E-Stop</button>
  {:else if state === 'off'}
    <button class="btn primary" onclick={() => send({ cmd: 'power', on: true })}>⏻ Power On</button>
  {:else if state === 'on'}
    <button class="btn primary" onclick={() => send({ cmd: 'home' })}>⌂ Home All</button>
  {:else if state === 'idle'}
    <button class="btn" onclick={() => send({ cmd: 'power', on: false })}>⏻ Power Off</button>
  {/if}

  <button class="estop" onclick={() => send({ cmd: 'estop' })} aria-label="Emergency stop">
    STOP
  </button>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 14px;
    background: var(--panel);
    border: 1px solid var(--hairline);
    border-radius: var(--r-lg);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--text);
  }

  .wordmark {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: 0.34em;
  }

  .state {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.1em;
    border: 1px solid var(--hairline);
    color: var(--text-dim);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
  }

  .state.live {
    color: var(--accent);
    border-color: rgba(39, 224, 255, 0.4);
    background: var(--accent-dim);
  }

  .state.live .dot {
    box-shadow: 0 0 8px var(--accent);
    animation: pulse 1.6s infinite;
  }

  .state.ok {
    color: var(--ok);
    border-color: rgba(52, 211, 153, 0.35);
  }

  .state.warn {
    color: var(--warn);
    border-color: rgba(251, 191, 36, 0.35);
  }

  .state.danger {
    color: var(--danger);
    border-color: rgba(255, 59, 59, 0.45);
    background: var(--danger-dim);
  }

  .state.dim {
    color: var(--text-dim);
  }

  @keyframes pulse {
    50% {
      opacity: 0.4;
    }
  }

  .alarm {
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--warn);
    font-size: 12px;
  }

  .spacer {
    flex: 1;
  }

  .conn {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--danger);
  }

  .conn.online {
    color: var(--text-faint);
  }

  .estop {
    padding: 10px 20px;
    border-radius: var(--r-md);
    background: var(--danger);
    color: #fff;
    font-weight: 800;
    font-size: 13px;
    letter-spacing: 0.14em;
    box-shadow: 0 0 0 1px rgba(255, 59, 59, 0.5), 0 4px 18px rgba(255, 59, 59, 0.25);
    transition: transform 80ms var(--ease), box-shadow 150ms var(--ease);
  }

  .estop:active {
    transform: scale(0.96);
  }
</style>
