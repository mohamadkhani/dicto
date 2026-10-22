import { LitElement, html } from 'lit';

/**
 * Clickable seek bar. Track + fill; click position maps to fraction (0..1).
 * The visible bar is 6px but the click zone is 18px (padded) so it's easy
 * to hit. Light DOM (no shadow root) so Tailwind classes apply directly.
 * Emits `seek` with {detail: fraction}.
 */
export class DictoSeek extends LitElement {
  static properties = {
    /** progress fraction 0..1 */
    value: { type: Number },
  };
  constructor() {
    super();
    this.value = 0;
  }
  createRenderRoot() { return this; }
  seek(e) {
    const r = this.querySelector('.track').getBoundingClientRect();
    const fraction = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    this.value = fraction;
    this.dispatchEvent(new CustomEvent('seek', { detail: fraction, bubbles: true, composed: true }));
  }
  render() {
    return html`
      <div class="inline-block w-30 py-1.5 cursor-pointer" @click=${this.seek}>
        <div class="track relative w-full h-1.5 rounded-full bg-elevated border border-line overflow-hidden">
          <div
            class="absolute top-0 left-0 bottom-0 bg-primary rounded-full transition-[width] duration-100 ease-linear"
            style="width:${(this.value * 100).toFixed(1)}%"
          ></div>
        </div>
      </div>`;
  }
}
customElements.define('dicto-seek', DictoSeek);
