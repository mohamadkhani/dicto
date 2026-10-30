import { LitElement, html } from 'lit';
import './dicto-section.js';
import './dicto-text-block.js';
import './dicto-button.js';
import './dicto-options.js';

/**
 * The Quick Translate popup card.
 *
 * state: idle | loading | ready | error | toolong
 * Two independent playback slots: `src` (Original) and `tr` (Translation),
 * each: { playback: idle|loading|playing|paused|ended, value: 0..1 }
 *
 * UX rules:
 * - One window frame: the title bar (title left, close X right) is the
 *   window's top strip — attached to the window border like a normal
 *   popup window, separated from content by a border. The body scrolls;
 *   the title bar and footer stay fixed.
 * - The Translate button is always present (disabled while loading) so the
 *   layout never jumps between states and a retry is always one click away.
 * - The Options footer is a full-bleed strip at the card bottom: the toggle
 *   on top, the panel expanding BELOW it (disclosure order), clipped by the
 *   card's rounded corners.
 * - Light DOM (no shadow root) so Tailwind classes apply directly.
 */
export class DictoPopup extends LitElement {
  static properties = {
    state: { type: String },
    original: { type: String },
    translation: { type: String },
    provider: { type: String },
    model: { type: String },
    error: { type: String },
    src: { type: Object }, // playback slot for Original
    tr: { type: Object },  // playback slot for Translation
    optionsOpen: { type: Boolean },
  };
  constructor() {
    super();
    this.state = 'idle';
    this.original = 'The quick brown fox jumps over the lazy dog. Select any text on your screen and press the hotkey to translate it.';
    this.translation = '';
    this.provider = 'GLM-4.7';
    this.model = 'OpenAI-compatible';
    this.error = 'API error: HTTP 429 Too Many Requests';
    this.src = { playback: 'idle', value: 0 };
    this.tr = { playback: 'idle', value: 0 };
    this.optionsOpen = false;
  }
  createRenderRoot() { return this; }

  toggleOptions() {
    this.optionsOpen = !this.optionsOpen;
  }

  originalSection() {
    return html`
      <dicto-section label="Original" .playback=${this.src.playback} .value=${this.src.value}></dicto-section>
      <dicto-text-block .text=${this.original}></dicto-text-block>
      <div class="h-px bg-line"></div>`;
  }

  render() {
    const busy = this.state === 'loading';

    let content;
    if (this.state === 'loading') {
      content = html`
        ${this.originalSection()}
        <div class="flex items-center gap-2 text-[13px] text-muted py-0.5">
          <span class="inline-flex gap-0.75 dots"><i></i><i></i><i></i></span>
          Translating…
        </div>`;
    } else if (this.state === 'ready') {
      // No "via provider · model" meta line — the Options footer summary
      // already shows the active provider.
      content = html`
        ${this.originalSection()}
        <dicto-section label="Translation" .playback=${this.tr.playback} .value=${this.tr.value}></dicto-section>
        <dicto-text-block emphasize .text=${this.translation}></dicto-text-block>`;
    } else if (this.state === 'error') {
      content = html`
        ${this.originalSection()}
        <span class="text-xs text-danger leading-normal">Translation failed: ${this.error}</span>`;
    } else if (this.state === 'toolong') {
      // Selection exceeds the 10,000-char limit. A warning, not an error:
      // nothing failed — the user just needs a smaller selection.
      const len = this.original.length;
      content = html`
        ${this.originalSection()}
        <div class="flex gap-2.5 items-start p-2.5 mb-2 border border-line border-l-2 border-l-warn rounded-md bg-bg text-ink text-xs leading-normal">
          <span class="text-warn text-sm">⚠</span>
          <div>
            <strong class="font-semibold">Selection is too long</strong>
            <div class="text-muted mt-0.5">
              ${len.toLocaleString()} characters — the limit is 10,000.
              Select a shorter passage and press the hotkey again.
            </div>
          </div>
        </div>`;
    } else {
      content = this.originalSection();
    }

    return html`
      <!-- ONE window frame: title strip attached to the window border
           (title + close), body, footer. Like a normal popup window. -->
      <div class="window w-115 max-h-140 flex flex-col
                  bg-surface border border-line rounded-[10px]
                  shadow-[0_8px_32px_rgba(0,0,0,0.5)] box-border">
        <!-- Title strip: attached to the window border, divided from the
             body by a border. Fixed while the body scrolls. -->
        <div class="flex items-center justify-between px-2 h-8 border-b border-line select-none">
          <span class="text-[11px] text-muted">Quick Translate</span>
          <button
            type="button"
            class="inline-flex items-center justify-center w-6 h-6 rounded-md cursor-pointer
                   text-muted transition-colors duration-150 hover:text-ink hover:bg-hover"
            title="Close (Esc)"
            aria-label="Close"
            @click=${() => this.dispatchEvent(new CustomEvent('close', { bubbles: true, composed: true }))}
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
              <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
            </svg>
          </button>
        </div>
        <div class="body flex-1 min-h-0 flex flex-col gap-2 overflow-y-auto px-3.5 py-3
                    [scrollbar-width:thin] [scrollbar-color:var(--color-line)_transparent]">
          ${content}
          <!-- Translate is always present; disabled while loading.
               Hidden in the toolong state — there's nothing to translate. -->
          ${this.state !== 'toolong'
            ? html`
              <div class="flex justify-center my-0.5 mb-0.5">
                <dicto-button variant="primary" label=${busy ? 'Translating…' : this.state === 'ready' ? 'Translate again' : 'Translate'} ?disabled=${busy}></dicto-button>
              </div>`
            : ''}
        </div>
        <!-- Footer region owns the window's bottom edge: full-bleed top
             border, bottom rounding, toggle above the panel it reveals. -->
        <div class="border-t border-line rounded-b-[9px] overflow-hidden">
          <div
            class="flex items-center gap-1.5 px-3.5 py-2 text-muted text-xs cursor-pointer select-none
                   transition-colors duration-150 hover:text-ink"
            role="button"
            aria-expanded=${this.optionsOpen}
            @click=${this.toggleOptions}
          >
            <span class="chevron inline-flex transition-transform duration-150 ${this.optionsOpen ? 'rotate-90' : ''}">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </span>
            Options
            <span class="flex-1"></span>
            <span class="text-[11px] text-muted">${this.provider} · Ctrl+Alt+D</span>
          </div>
          ${this.optionsOpen ? html`<dicto-options .open=${this.optionsOpen}></dicto-options>` : ''}
        </div>
      </div>

      <style>
        @keyframes blink { 0%, 80%, 100% { opacity: 0.2; } 40% { opacity: 1; } }
        .dots { display: inline-flex; gap: 3px; }
        .dots i {
          width: 5px; height: 5px; border-radius: 3px;
          background: var(--color-muted);
          animation: blink 1.2s infinite both;
        }
        .dots i:nth-child(2) { animation-delay: 0.2s; }
        .dots i:nth-child(3) { animation-delay: 0.4s; }
      </style>`;
  }
}
customElements.define('dicto-popup', DictoPopup);
