import { createElement, useEffect, useState } from "react";

/** Use the CS home logo engine, with an SVG fallback while its module loads. */
export function ServiceLogo({ state, src, morph = false }: { state: string; src: string; morph?: boolean }) {
  const [ready, setReady] = useState(false);
  const tag = morph ? "cs-morph-logo" : "cs-brand-logo";

  useEffect(() => {
    let active = true;
    const engine = morph ? import("../../brand/engine/cs-morph-logo.js") : import("../../brand/engine/cs-logo-engine.js");
    void engine.then(() => {
      if (active && customElements.get(tag)) setReady(true);
    }).catch(() => { /* Keep the approved static mark if the engine cannot load. */ });
    return () => { active = false; };
  }, [morph, tag]);

  return ready
    ? createElement(tag, { state: state === "master-mark" ? "master" : state, label: "", "aria-hidden": "true" })
    : <img src={src} alt="" />;
}
