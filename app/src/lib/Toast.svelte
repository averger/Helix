<script>
  import { rejection } from './store.js'
</script>

{#if $rejection}
  <div class="toast" role="alert">
    <span class="cmd">{$rejection.cmd}</span>
    {$rejection.reason}
  </div>
{/if}

<style>
  .toast {
    position: absolute;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 18px;
    border-radius: var(--r-md);
    background: rgba(13, 15, 18, 0.92);
    border: 1px solid rgba(251, 191, 36, 0.4);
    color: var(--warn);
    font-size: 13px;
    backdrop-filter: blur(12px);
    animation: rise 200ms var(--ease);
    z-index: 20;
    max-width: 70%;
  }

  @keyframes rise {
    from {
      transform: translate(-50%, 8px);
      opacity: 0;
    }
  }

  .cmd {
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid rgba(251, 191, 36, 0.4);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
</style>
