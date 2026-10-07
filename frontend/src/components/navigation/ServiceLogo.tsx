import { createElement, useEffect, useState } from "react";

/** Use the CS home marks without its decorative lightning effects in services. */
export function ServiceLogo({ state, src }: { state: string; src: string }) {
  const [ready, setReady] = useState(false);
  const tag = "cs-brand-logo";

  useEffect(() => {
    let active = true;
    const engine = import("../../brand/engine/cs-logo-engine.js");
    void engine.then(() => {
      if (active && customElements.get(tag)) setReady(true);
    }).catch(() => { /* Keep the approved static mark if the engine cannot load. */ });
    return () => { active = false; };
  }, []);

  return ready
    ? createElement(tag, { state: state === "master-mark" ? "master" : state, label: "", "aria-hidden": "true" })
    : <img src={src} alt="" />;
}
