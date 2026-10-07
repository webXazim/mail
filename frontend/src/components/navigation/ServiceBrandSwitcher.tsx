import { useEffect, useId, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { useLocation } from "react-router-dom";
import { ServiceLogo } from "./ServiceLogo";
import masterLogo from "../../assets/logos/cs-master-mark.svg";
import mailLogo from "../../assets/logos/cs-mail.svg";
import mailerLogo from "../../assets/logos/cs-mailer.svg";
import docsLogo from "../../assets/logos/cs-docs.svg";
import connectLogo from "../../assets/logos/cs-connect.svg";
import notesLogo from "../../assets/logos/cs-notes.svg";
import keylangLogo from "../../assets/logos/cs-keylang.svg";
import "../../styles/components/service-brand-switcher.css";

export const CONNECT_BRAND_LOGO = connectLogo;
const brandLogos = { "master-mark": masterLogo, mail: mailLogo, mailer: mailerLogo, docs: docsLogo, connect: connectLogo, notes: notesLogo, keylang: keylangLogo };

export const SERVICE_BRANDS = [
  { key: "master-mark", label: "CrescentSphere", description: "Product family", href: "https://crescentsphere.com" },
  { key: "mail", label: "CS Mail", description: "Business email", href: "https://mail.crescentsphere.com" },
  { key: "mailer", label: "CS Mailer", description: "Transactional email", href: "https://mailer.crescentsphere.com" },
  { key: "docs", label: "CS Docs", description: "Operations", href: "https://docs.crescentsphere.com" },
  { key: "connect", label: "CS Connect", description: "Communication", href: "https://connect.crescentsphere.com" },
  { key: "notes", label: "CS Notes", description: "Secure notes", href: "https://notes.crescentsphere.com" },
  { key: "keylang", label: "CS KeyLang", description: "Typing & language", href: "https://keylang.crescentsphere.com" },
] as const;

type ServiceKey = keyof typeof brandLogos;

// Read tokens at the trigger because some services scope their theme to the app,
// while the floating menu is rendered under document.body.
const themeTokens: Record<Exclude<ServiceKey, "master-mark">, readonly [string, string, string, string, string, string]> = {
  connect: ["--ms-color-surface", "--ms-color-ink", "--ms-color-ink-muted", "--ms-color-line", "--ms-color-surface-subtle", "--ms-color-ink"],
  mail: ["--cs-mail-panel-raised", "--cs-mail-paper", "--cs-mail-copy", "--cs-mail-line", "--cs-mail-panel-soft", "--cs-mail-accent"],
  mailer: ["--surface", "--text", "--muted", "--line", "--surface-soft", "--blue"],
  docs: ["--ui-v2-surface", "--ui-v2-text", "--ui-v2-text-secondary", "--ui-v2-border", "--ui-v2-surface-hover", "--ui-v2-border-focus"],
  notes: ["--surface", "--foreground", "--muted", "--border", "--surface-hover", "--focus-ring"],
  keylang: ["--surface", "--ink", "--muted", "--line", "--surface-muted", "--teal"],
};
const themeProperties = ["--service-menu-surface", "--service-menu-text", "--service-menu-muted", "--service-menu-line", "--service-menu-hover", "--service-menu-focus"];
const themeFallbacks = ["#ffffff", "#171817", "#5f625f", "#e4e5e3", "#f5f5f4", "#989d98"];

type ServiceBrandSwitcherProps = { activeService?: ServiceKey; compact?: boolean; mobile?: boolean; onOpen?: () => void };

export function ServiceBrandSwitcher(props: ServiceBrandSwitcherProps) {
  const location = useLocation();
  // A route change starts a fresh menu and cleans up the old panel's listeners.
  return <ServiceBrandMenu key={JSON.stringify([location.pathname, location.search])} {...props} />;
}

function ServiceBrandMenu({ activeService = "connect", compact = false, mobile = false, onOpen }: ServiceBrandSwitcherProps) {
  const activeBrand = SERVICE_BRANDS.find((brand) => brand.key === activeService)!;
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ top: 12, left: 12, width: 960 });
  const [panelTheme, setPanelTheme] = useState<CSSProperties>({});
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  useEffect(() => {
    if (!open) return;
    const positionPanel = () => {
      const trigger = triggerRef.current?.getBoundingClientRect();
      if (!trigger) return;
      const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
      const width = Math.min(960, viewportWidth - 24);
      const height = Math.min(panelRef.current?.offsetHeight ?? 80, window.innerHeight - 24);
      const beside = viewportWidth >= 1100 && trigger.right + 12 + width <= viewportWidth - 12;
      setPosition({
        width,
        left: Math.max(12, Math.min(beside ? trigger.right + 12 : trigger.left, viewportWidth - width - 12)),
        top: Math.max(12, Math.min(beside ? trigger.top + (trigger.height - height) / 2 : mobile ? trigger.top - height - 8 : trigger.bottom + 8, window.innerHeight - height - 12)),
      });
    };
    const syncTheme = () => {
      if (!triggerRef.current) return;
      const style = getComputedStyle(triggerRef.current);
      const tokens = themeTokens[activeService === "master-mark" ? "docs" : activeService];
      setPanelTheme(Object.fromEntries(themeProperties.map((property, index) => [property, style.getPropertyValue(tokens[index] ?? "").trim() || themeFallbacks[index]])) as CSSProperties);
    };
    syncTheme();
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    colorScheme.addEventListener("change", syncTheme);
    const themeObserver = new MutationObserver(syncTheme);
    for (let ancestor: HTMLElement | null = triggerRef.current; ancestor; ancestor = ancestor.parentElement) {
      themeObserver.observe(ancestor, { attributes: true, attributeFilter: ["class", "style", "data-theme"] });
    }
    positionPanel();
    panelRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    const dismiss = (event: PointerEvent | FocusEvent) => {
      if (event.target instanceof Node && !panelRef.current?.contains(event.target) && !triggerRef.current?.contains(event.target)) setOpen(false);
    };
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("focusin", dismiss);
    document.addEventListener("keydown", escape);
    window.addEventListener("resize", positionPanel);
    window.addEventListener("scroll", positionPanel, true);
    return () => {
      themeObserver.disconnect();
      colorScheme.removeEventListener("change", syncTheme);
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("focusin", dismiss);
      document.removeEventListener("keydown", escape);
      window.removeEventListener("resize", positionPanel);
      window.removeEventListener("scroll", positionPanel, true);
    };
  }, [open, mobile, activeService]);

  const showMenu = () => { onOpen?.(); setOpen(true); };
  const navigateMenu = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(panelRef.current?.querySelectorAll<HTMLAnchorElement>('[role="menuitem"]') ?? []);
    const index = items.indexOf(document.activeElement as HTMLAnchorElement);
    let next: number;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") next = (index + 1) % items.length;
    else if (event.key === "ArrowUp" || event.key === "ArrowLeft") next = (index - 1 + items.length) % items.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = items.length - 1;
    else return;
    event.preventDefault();
    items[next]?.focus();
  };

  return (
    <div className={`service-brand-switcher${compact ? " service-brand-switcher--compact" : ""}${mobile ? " service-brand-switcher--mobile" : ""}`}>
      <button ref={triggerRef} type="button" className="service-brand-switcher__trigger" aria-label={`${activeBrand.label}. Switch service`} aria-haspopup="menu" aria-expanded={open} aria-controls={open ? menuId : undefined}
        onClick={() => { if (open) { setOpen(false); triggerRef.current?.focus(); } else showMenu(); }}
        onKeyDown={(event) => { if (["ArrowDown", "ArrowUp", "ArrowRight", "ArrowLeft"].includes(event.key)) { event.preventDefault(); showMenu(); } }}>
        <span className="service-brand-switcher__mark"><ServiceLogo state={activeService} src={brandLogos[activeService]} morph /></span>
        {!compact && <strong>{activeBrand.label}</strong>}
        {mobile && <span className="ms-navigation__label">Services</span>}
      </button>
      {open && createPortal(
        <div ref={panelRef} id={menuId} className="service-brand-switcher__panel" role="menu" aria-label="Switch CrescentSphere service" style={{ ...position, ...panelTheme }} onKeyDown={navigateMenu}>
          {SERVICE_BRANDS.filter((brand) => brand.key !== activeService).map((brand) => (
            <a key={brand.key} className="service-brand-switcher__item" role="menuitem" aria-label={brand.label} aria-description={brand.description}
              href={brand.key === "connect" ? "https://connect.crescentsphere.com" : brand.href} onClick={() => setOpen(false)}>
              <span className="service-brand-switcher__mark"><ServiceLogo state={brand.key} src={brandLogos[brand.key]} /></span>
              <span><strong>{brand.label}</strong><small>{brand.description}</small></span>
            </a>
          ))}
        </div>, document.body,
      )}
    </div>
  );
}
