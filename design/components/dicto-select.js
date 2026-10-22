import { LitElement, html } from 'lit';

/**
 * A dropdown selector used in the Options panel (provider / model / target /
 * TTS / voice). A styled native <select>: the browser provides the dropdown
 * list, the theme provides the chrome. Light DOM (no shadow root) so Tailwind
 * classes apply directly.
 *
 * Props:
 *   items: [{ id, label }]  — options; ids are committed values, labels shown
 *   value: string           — the selected id ('' = nothing selected)
 *   placeholder: string     — shown while nothing is selected
 *
 * Fires a bubbling `select` event with { detail: { value } } on change.
 */
export class DictoSelect extends LitElement {
  static properties = {
    items: { type: Array },
    value: { type: String, reflect: true },
    placeholder: { type: String },
  };
  constructor() {
    super();
    this.items = [];
    this.value = '';
    this.placeholder = '';
  }
  createRenderRoot() { return this; }
  onChange(e) {
    this.value = e.target.value;
    this.dispatchEvent(
      new CustomEvent('select', { bubbles: true, composed: true, detail: { value: this.value } })
    );
  }
  render() {
    return html`
      <div class="relative flex-1 min-w-0">
        <select
          class="w-full appearance-none bg-elevated text-ink text-[11px] leading-normal
                 border border-line rounded-md pl-2 pr-6 py-[4px] cursor-pointer
                 hover:bg-hover focus:outline-none focus:border-primary transition-colors"
          @change=${(e) => this.onChange(e)}
        >
          ${this.placeholder
            ? html`<option value="" ?disabled=${!!this.value} ?selected=${!this.value} class="text-muted">
                ${this.placeholder}
              </option>`
            : ''}
          ${this.items.map(
            (it) => html`<option value=${it.id} ?selected=${it.id === this.value}>${it.label}</option>`
          )}
        </select>
        <svg
          class="pointer-events-none absolute right-1.5 top-1/2 -translate-y-1/2 text-muted"
          width="10" height="10" viewBox="0 0 16 16" fill="none"
        >
          <path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </div>`;
  }
}
customElements.define('dicto-select', DictoSelect);
