import masterMark from "../../assets/logos/cs-master-mark.svg";
import masterFull from "../../assets/logos/cs-master.svg";
import mail from "../../assets/logos/cs-mail.svg";
import mailer from "../../assets/logos/cs-mailer.svg";
import docs from "../../assets/logos/cs-docs.svg";
import connect from "../../assets/logos/cs-connect.svg";
import notes from "../../assets/logos/cs-notes.svg";
import keylang from "../../assets/logos/cs-keylang.svg";
const logoUrls = { "cs-master-mark.svg": masterMark, "cs-master.svg": masterFull, "cs-mail.svg": mail, "cs-mailer.svg": mailer, "cs-docs.svg": docs, "cs-connect.svg": connect, "cs-notes.svg": notes, "cs-keylang.svg": keylang };
(() => {
  'use strict';

  const STATES = Object.freeze({
    master: { label: 'CrescentSphere', file: 'cs-master-mark.svg', fullFile: 'cs-master.svg' },
    mail: { label: 'CS Mail', file: 'cs-mail.svg' },
    mailer: { label: 'CS Mailer', file: 'cs-mailer.svg' },
    docs: { label: 'CS Docs', file: 'cs-docs.svg' },
    connect: { label: 'CS Connect', file: 'cs-connect.svg' },
    notes: { label: 'CS Notes', file: 'cs-notes.svg' },
    keylang: { label: 'CS KeyLang', file: 'cs-keylang.svg' }
  });

  const ALIASES = Object.freeze({ overview: 'master', hero: 'master', dev: 'mailer', cta: 'master', footer: 'master' });

  const normalizeState = (value) => {
    const key = String(value || 'master').toLowerCase();
    const resolved = ALIASES[key] || key;
    return STATES[resolved] ? resolved : 'master';
  };
  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

  class CSBrandLogo extends HTMLElement {
    static get observedAttributes() { return ['state', 'variant', 'label']; }

    constructor() {
      super();
      this.attachShadow({ mode: 'open' });
      this._currentState = null;
      this._token = 0;
      this.shadowRoot.innerHTML = `
        <style>
          :host {
            display: inline-block;
            width: 1em;
            height: 1em;
            line-height: 0;
            vertical-align: middle;
            contain: content;
          }
          .frame {
            position: relative;
            width: 100%;
            height: 100%;
            display: grid;
            place-items: center;
          }
          img {
            grid-area: 1 / 1;
            display: block;
            width: 100%;
            height: 100%;
            object-fit: contain;
            opacity: 0;
            transform: scale(.985);
            transition: opacity 220ms ease, transform 260ms cubic-bezier(.2,.75,.2,1);
            pointer-events: none;
            user-select: none;
          }
          img.is-current {
            opacity: 1;
            transform: scale(1);
          }
          @media (prefers-reduced-motion: reduce) {
            img { transition: none !important; }
          }
        </style>
        <span class="frame" part="frame">
          <img class="layer-a is-current" alt="" aria-hidden="true" />
          <img class="layer-b" alt="" aria-hidden="true" />
        </span>`;
      this._a = this.shadowRoot.querySelector('.layer-a');
      this._b = this.shadowRoot.querySelector('.layer-b');
      this._front = this._a;
      this._back = this._b;
    }

    connectedCallback() {
      if (this.getAttribute('aria-hidden') === 'true') {
        this.removeAttribute('role');
        this.removeAttribute('aria-label');
      } else if (!this.hasAttribute('role')) {
        this.setAttribute('role', 'img');
      }
      this._applyState(this.state, true);
    }

    attributeChangedCallback(name, oldValue, newValue) {
      if (!this.isConnected || oldValue === newValue) return;
      if (name === 'state' || name === 'variant') this._applyState(this.state, this.hasAttribute('data-instant'));
      if (name === 'label') this._syncAria();
    }

    get state() {
      return normalizeState(this.getAttribute('state'));
    }

    set state(value) {
      this.setAttribute('state', value);
    }

    get variant() {
      return this.getAttribute('variant') === 'full' ? 'full' : 'mark';
    }

    _srcFor(state) {
      const meta = STATES[state] || STATES.master;
      const file = state === 'master' && this.variant === 'full' ? meta.fullFile : meta.file;
      return logoUrls[file];
    }

    _syncAria() {
      if (this.getAttribute('aria-hidden') === 'true') return;
      const explicit = this.getAttribute('label');
      this.setAttribute('aria-label', explicit || STATES[this.state].label);
    }

    _applyState(state, instant) {
      const next = normalizeState(state);
      this._syncAria();
      const src = this._srcFor(next);
      if (this._currentState === next && this._front.getAttribute('src') === src) return;

      const token = ++this._token;
      const commit = () => {
        if (token !== this._token) return;
        if (instant || reducedMotion.matches || !this._front.getAttribute('src')) {
          this._front.src = src;
          this._front.classList.add('is-current');
          this._back.classList.remove('is-current');
          this._currentState = next;
          this.dispatchEvent(new CustomEvent('cs-logo-statechange', { detail: { state: next }, bubbles: true }));
          return;
        }

        this._back.src = src;
        requestAnimationFrame(() => {
          if (token !== this._token) return;
          this._back.classList.add('is-current');
          this._front.classList.remove('is-current');
          const oldFront = this._front;
          this._front = this._back;
          this._back = oldFront;
          this._currentState = next;
          window.setTimeout(() => {
            if (token === this._token) this._back.removeAttribute('src');
          }, 290);
          this.dispatchEvent(new CustomEvent('cs-logo-statechange', { detail: { state: next }, bubbles: true }));
        });
      };

      const preload = new Image();
      preload.onload = commit;
      preload.onerror = commit;
      preload.src = src;
    }
  }

  if (!customElements.get('cs-brand-logo')) customElements.define('cs-brand-logo', CSBrandLogo);

})();
