import { LitElement, html } from 'lit';

/**
 * Buttons used across the popup. Light DOM (no shadow root) so Tailwind
 * classes apply directly.
 *
 * NOTE: light DOM means <slot> doesn't project children — the label is a
 * property (`label="Translate"`), not slotted text.
 *
 * variant: 'primary' (Translate), 'ghost' (text), 'icon' (square glyph)
 */
export class DictoButton extends LitElement {
  static properties = {
    variant: { type: String },
    label: { type: String },
    disabled: { type: Boolean, reflect: true },
    title: { type: String },
  };
  constructor() {
    super();
    this.variant = 'ghost';
    this.label = '';
    this.disabled = false;
    this.title = '';
  }
  createRenderRoot() { return this; }
  render() {
    const base = 'inline-flex items-center gap-1 rounded-md cursor-pointer font-sans transition-[colors,opacity] duration-150 focus-visible:outline focus-visible:outline-1 focus-visible:outline-primary';
    const variants = {
      // the main Translate action
      primary: 'px-4 py-[6px] bg-primary text-bg text-xs font-semibold hover:opacity-90 disabled:opacity-45 disabled:cursor-default',
      // text ghost
      ghost: 'px-[9px] py-1 bg-elevated text-muted border border-line text-xs leading-none hover:bg-hover hover:text-ink disabled:opacity-45 disabled:cursor-default',
      // icon-only ghost (▶ ⏸ ↺): square hit area
      icon: 'px-[7px] py-1 bg-elevated text-muted border border-line text-[13px] leading-none hover:bg-hover hover:text-ink disabled:opacity-45 disabled:cursor-default',
    };
    return html`
      <button
        type="button"
        class="${base} ${variants[this.variant] || variants.ghost}"
        ?disabled=${this.disabled}
        title=${this.title || this.label}
        @click=${() => this.dispatchEvent(new CustomEvent('action', { bubbles: true, composed: true }))}
      >${this.label}</button>`;
  }
}
customElements.define('dicto-button', DictoButton);
