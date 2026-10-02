import { LitElement, html } from 'lit';
import './dicto-button.js';
import './dicto-seek.js';

/**
 * A section header row: small uppercase label + ONE contextual play button +
 * a status indicator + (when a clip is loaded) a seek bar.
 *
 * The single button is contextual:
 *  idle    → ▶            (will synthesize from the AI TTS API)
 *  loading → ◌ spinner    (synthesizing; disabled)
 *  playing → ⏸            (pause)
 *  paused  → ▶            (resume)
 *  ended   → ↺            (replay from start — no re-synthesis)
 *
 * The status indicator (next to the label) tells the user what the button
 * will do before a clip exists:
 *  · "AI TTS"         — text is new; pressing play synthesizes via the API
 *  · "synthesizing…"  — in flight
 *  · "cached · replay" — clip already loaded
 *
 * Light DOM (no shadow root) so Tailwind classes apply directly.
 */
export class DictoSection extends LitElement {
  static properties = {
    label: { type: String },
    /** idle | loading | playing | paused | ended */
    playback: { type: String },
    /** seek position fraction 0..1 */
    value: { type: Number },
  };
  constructor() {
    super();
    this.label = '';
    this.playback = 'idle';
    this.value = 0;
  }
  createRenderRoot() { return this; }
  render() {
    const { playback } = this;
    const hasClip = ['playing', 'paused', 'ended'].includes(playback);
    const value = playback === 'ended' ? 1 : this.value;

    let glyph, aria, disabled = false;
    if (playback === 'idle')         { glyph = '▶';   aria = 'Speak (AI TTS)'; }
    else if (playback === 'loading') { glyph = '◌';   aria = 'Synthesizing…'; disabled = true; }
    else if (playback === 'playing') { glyph = '⏸';   aria = 'Pause'; }
    else if (playback === 'paused')  { glyph = '▶';   aria = 'Resume'; }
    else                             { glyph = '↺';   aria = 'Replay'; }

    let status = '';
    if (playback === 'idle')         status = html`<span class="text-[10px] px-1.5 py-px rounded text-primary border border-line whitespace-nowrap">AI TTS</span>`;
    else if (playback === 'loading') status = html`<span class="text-[10px] px-1.5 py-px rounded text-muted bg-elevated whitespace-nowrap">synthesizing…</span>`;
    else if (playback === 'ended')   status = html`<span class="text-[10px] px-1.5 py-px rounded text-muted bg-elevated whitespace-nowrap">cached · replay</span>`;

    return html`
      <span class="flex justify-between items-center gap-2">
        <span class="flex items-center gap-1.5 min-w-0">
          <span class="text-[11px] tracking-wider uppercase text-muted whitespace-nowrap">${this.label}</span>
          ${status}
        </span>
        <span class="flex items-center gap-1.5 ml-auto">
          <dicto-button variant="icon" label=${glyph} ?disabled=${disabled} title=${aria}></dicto-button>
          ${hasClip ? html`<dicto-seek .value=${value}></dicto-seek>` : ''}
        </span>
      </span>`;
  }
}
customElements.define('dicto-section', DictoSection);
