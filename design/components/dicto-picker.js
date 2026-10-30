import { LitElement, html } from 'lit';
import './dicto-select.js';

/**
 * Adaptive option picker: chips for lists of up to `chipsUpTo` options, a
 * dropdown select beyond it. Both modes fire the same bubbling `select`
 * event with { detail: { value } }, so owners never care which mode is
 * active.
 *
 * Props:
 *   items: [{ id, label }]  — options; ids are committed values, labels shown
 *   value: string           — the selected id ('' = nothing selected)
 *   chipsUpTo: Number       — chips while items.length <= this (default 4)
 *   placeholder: string     — dropdown hint while nothing is selected
 *
 * Light DOM (no shadow root) so Tailwind classes apply directly.
 */
export class DictoPicker extends LitElement {
  static properties = {
    items: { type: Array },
    value: { type: String, reflect: true },
    chipsUpTo: { type: Number },
    placeholder: { type: String },
  };
  constructor() {
    super();
    this.items = [];
    this.value = '';
    this.chipsUpTo = 4;
    this.placeholder = '';
  }
  createRenderRoot() { return this; }
  // Re-emit from THIS element so listeners route by the picker (data-key),
  // not by whichever inner control happened to render.
  onSelect(value) {
    this.value = value;
    this.dispatchEvent(
      new CustomEvent('select', { bubbles: true, composed: true, detail: { value } })
    );
  }
  render() {
    if (this.items.length > this.chipsUpTo) {
      return html`
        <dicto-select
          .items=${this.items}
          .value=${this.value}
          placeholder=${this.placeholder}
          @select=${(e) => this.onSelect(e.detail.value)}
        ></dicto-select>`;
    }
    return html`
      <div class="flex flex-wrap gap-1 pt-0.5">
        ${this.items.map(
          (it) => html`
            <button
              type="button"
              class="px-2 py-[3px] rounded text-[11px] leading-normal whitespace-nowrap cursor-pointer transition-colors
                     ${it.id === this.value
                       ? 'bg-primary text-bg font-semibold'
                       : 'bg-elevated text-muted hover:bg-hover hover:text-ink border border-line'}"
              @click=${() => this.onSelect(it.id)}
            >${it.label}</button>`
        )}
      </div>`;
  }
}
customElements.define('dicto-picker', DictoPicker);
