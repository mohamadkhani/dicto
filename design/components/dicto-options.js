import { LitElement, html } from 'lit';
import './dicto-picker.js';

/**
 * The inline Options panel: adaptive pickers for
 * Provider → Model, Target language, TTS preset → Voice.
 * Each picker renders chips for lists of up to `chipsUpTo` options and a
 * dropdown beyond it — so short lists (provider, TTS presets, Anthropic's
 * models) stay chips, long ones (13 languages, OpenAI models, preset
 * voices) collapse into dropdowns. Two-level: choosing a
 * provider/TTS preset swaps the model/voice option lists. Hand-edited
 * values outside the catalog appear as a "Custom: …" option.
 * Light DOM (no shadow root) so Tailwind classes apply directly.
 */
export class DictoOptions extends LitElement {
  static properties = {
    open: { type: Boolean, reflect: true },
    provider: { type: String },   // anthropic | openai
    model: { type: String },
    target: { type: String },
    tts: { type: String },        // grok | kokoro | openai
    voice: { type: String },
  };
  constructor() {
    super();
    this.open = false;
    this.provider = 'openai';
    this.model = 'GLM-5.3-flash';
    this.target = 'English';
    this.tts = 'grok';
    this.voice = 'Rex';
  }
  static models = {
    anthropic: [
      { id: 'claude-sonnet-4-6', label: 'Sonnet 4.6 (balanced)' },
      { id: 'claude-opus-4-7', label: 'Opus 4.7 (most capable)' },
      { id: 'claude-haiku-4-5', label: 'Haiku 4.5 (fast)' },
    ],
    openai: [
      { id: 'GLM-5.3-flash', label: 'GLM-5.3-flash (z.ai)' },
      { id: 'gpt-4o-mini', label: 'GPT-4o mini' },
      { id: 'gpt-5', label: 'GPT-5' },
      { id: 'llama-3.1-70b', label: 'Llama 3.1 70B' },
      { id: 'mistral-large', label: 'Mistral Large' },
    ],
  };
  static targets = ['English', 'Persian', 'Arabic', 'French', 'German', 'Spanish', 'Italian',
    'Russian', 'Chinese', 'Japanese', 'Korean', 'Turkish', 'Hindi'];
  static ttsPresets = [
    { id: 'grok', label: 'Grok Voice (OpenRouter)', voices: ['Eve', 'Ara', 'Rex', 'Sal', 'Leo'] },
    { id: 'kokoro', label: 'Kokoro (OpenRouter)', voices: ['bf_emma', 'bf_alice', 'bm_george', 'af_sky', 'am_adam'] },
    { id: 'openai', label: 'OpenAI', voices: ['alloy', 'nova', 'shimmer', 'echo', 'fable', 'onyx'] },
  ];
  /** Pickers render chips for lists of up to this many options. */
  static chipsUpTo = 4;
  createRenderRoot() { return this; }

  // Append a "Custom: …" option when the current value is off-catalog.
  withCustom(items, current) {
    if (current && !items.some((it) => it.id === current)) {
      items = [...items, { id: current, label: `Custom: ${current}` }];
    }
    return items;
  }
  modelItems() {
    return this.withCustom(DictoOptions.models[this.providerKey(this.provider)], this.model);
  }
  targetItems() {
    return this.withCustom(DictoOptions.targets.map((l) => ({ id: l, label: l })), this.target);
  }
  ttsItems() {
    return this.withCustom(
      DictoOptions.ttsPresets.map((p) => ({ id: p.id, label: p.label })),
      this.tts
    );
  }
  voiceItems() {
    const preset = DictoOptions.ttsPresets.find((p) => p.id === this.tts);
    const voices = (preset?.voices ?? []).map((v) => ({ id: v, label: v }));
    return this.withCustom(voices, this.voice);
  }
  providerKey(label) {
    return label === 'Anthropic' ? 'anthropic' : 'openai';
  }
  onSelect(e) {
    const { value } = e.detail;
    const key = e.composedPath()[0].closest('dicto-picker').dataset.key;
    if (key === 'provider') {
      this.provider = value;
      // Switching provider swaps the model catalog: land on its first model.
      // A list crossing this bound (3 Anthropic ↔ 5 OpenAI models) makes
      // the model picker switch chip ↔ dropdown on its own.
      this.model = DictoOptions.models[this.providerKey(value)][0].id;
    } else if (key === 'tts') {
      const preset = DictoOptions.ttsPresets.find((p) => p.id === value);
      this.tts = preset.id;
      this.voice = preset.voices[0];
    } else {
      this[key] = value;
    }
  }
  render() {
    const field = (key, items, value, placeholder) => html`
      <span class="text-[10px] tracking-wider uppercase text-muted pt-1.5 whitespace-nowrap">${key}</span>
      <dicto-picker
        data-key=${key.toLowerCase()}
        .items=${items}
        .value=${value}
        .chipsUpTo=${DictoOptions.chipsUpTo}
        placeholder=${placeholder ?? ''}
      ></dicto-picker>`;
    return html`
      <div
        class="grid grid-cols-[64px_1fr] gap-x-3 gap-y-2 items-start p-2.5 mx-3.5 mb-3
               bg-bg border border-line rounded-md"
        @select=${this.onSelect}
      >
        ${field('Provider', [
          { id: 'Anthropic', label: 'Anthropic' },
          { id: 'OpenAI-compatible', label: 'OpenAI-compatible' },
        ], this.provider === 'anthropic' ? 'Anthropic' : 'OpenAI-compatible')}
        ${field('Model', this.modelItems(), this.model)}
        ${field('Target', this.targetItems(), this.target, 'Select language…')}
        <div class="col-span-2 h-px bg-line"></div>
        ${field('TTS', this.ttsItems(), this.tts)}
        ${field('Voice', this.voiceItems(), this.voice, 'Select voice…')}
      </div>`;
  }
}
customElements.define('dicto-options', DictoOptions);
