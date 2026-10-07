import { useEffect, useState } from "react";

import { usePoll, type Route } from "../hooks";
import { PRICE_SOURCE, policyPrice } from "../policy";

/** Home page sections the bar links to; each is a <section id> on the home page. */
export const SECTIONS = [
  { id: "policies", label: "Policies" },
  { id: "try", label: "Try a loan" },
  { id: "how", label: "How it runs" },
  { id: "keys", label: "Keys" },
  { id: "private", label: "Privacy" },
  { id: "testnet", label: "Testnet" },
] as const;

/** The mark: the disc and still-water line from the favicon (web/DESIGN.md, Mark). */
export function LenticMark() {
  return (
    <svg className="mark" viewBox="0 0 32 32" aria-hidden>
      <circle cx="16" cy="16" r="14" fill="var(--mark)" />
      <path d="M6 18c3-2 5-2 10 0s7 2 10 0" stroke="#ffffff" strokeWidth="2.5" strokeLinecap="round" fill="none" />
    </svg>
  );
}

/** Scrolls to a home section once it is in the DOM (the home page may still be mounting). */
export function scrollToSection(id: string, smooth: boolean) {
  let frames = 0;
  const tick = () => {
    const el = document.getElementById(id);
    if (el) el.scrollIntoView({ behavior: smooth ? "smooth" : "auto", block: "start" });
    else if (frames++ < 30) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/** The section crossing the middle of the viewport, for the underline. Sections the bar has no link to clear it. */
function useActiveSection(enabled: boolean) {
  const [active, setActive] = useState<string | null>(null);
  useEffect(() => {
    if (!enabled) {
      setActive(null);
      return;
    }
    const linked = new Set<string>(SECTIONS.map((s) => s.id));
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) setActive(linked.has(e.target.id) ? e.target.id : null);
      },
      { rootMargin: "-45% 0px -50% 0px" },
    );
    document.querySelectorAll("main > section").forEach((el) => io.observe(el));
    return () => io.disconnect();
  }, [enabled]);
  return active;
}

/** "#/?s=<id>" scrolls to that home section; any other hash change starts the new page at the top. */
function useHashScroll(reduced: boolean) {
  useEffect(() => {
    const go = (smooth: boolean) => {
      const section = new URLSearchParams(window.location.hash.split("?")[1] ?? "").get("s");
      if (section) scrollToSection(section, smooth && !reduced);
      else if (!window.location.hash.includes("drill=")) window.scrollTo(0, 0);
    };
    if (window.location.hash.includes("s=")) go(false);
    const onHash = () => go(true);
    // A section link to the hash already in the address bar fires no hashchange.
    const onClick = (e: MouseEvent) => {
      const link = e.target instanceof Element ? e.target.closest("a[href^='#/?s=']") : null;
      if (link && link.getAttribute("href") === window.location.hash) {
        e.preventDefault();
        go(true);
      }
    };
    window.addEventListener("hashchange", onHash);
    document.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("hashchange", onHash);
      document.removeEventListener("click", onClick);
    };
  }, [reduced]);
}

export function NavBar({ route, reduced }: { route: Route; reduced: boolean }) {
  const [open, setOpen] = useState(false);
  const price = usePoll(policyPrice, [], 15_000);
  const active = useActiveSection(route.page === "home");
  useHashScroll(reduced);

  useEffect(() => setOpen(false), [route]);

  const sectionLinks = SECTIONS.map((s) => (
    <a
      key={s.id}
      href={`#/?s=${s.id}`}
      onClick={() => setOpen(false)}
      aria-current={route.page === "home" && active === s.id ? "location" : undefined}
    >
      {s.label}
    </a>
  ));

  return (
    <header className="top">
      <div className="top-inner">
        <a
          className="wordmark"
          href="#/"
          aria-label="Lentic, home"
          onClick={(e) => {
            // Already home: the hash does not change, so go to the top here.
            if (window.location.hash === "#/" || window.location.hash === "") {
              e.preventDefault();
              window.scrollTo({ top: 0, behavior: reduced ? "auto" : "smooth" });
            }
            setOpen(false);
          }}
        >
          <LenticMark />
          <span>Lentic</span>
        </a>
        <nav className="sections" aria-label="Sections">
          {sectionLinks}
        </nav>
        <div className="top-right">
          {price.data && (
            <span className="live-price" title={`${PRICE_SOURCE[price.data.kind]}, from the policy signer`}>
              <span className="live-label">ZEC/USD</span>
              <span className="live-value">
                {price.data.usd.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
              </span>
            </span>
          )}
          <a
            className="btn secondary book-link"
            href="#/book"
            aria-current={route.page === "book" ? "page" : undefined}
          >
            Lender book
          </a>
          <button
            type="button"
            className="btn secondary menu-toggle"
            aria-expanded={open}
            aria-controls="menu-panel"
            onClick={() => setOpen((o) => !o)}
          >
            {open ? "Close" : "Menu"}
          </button>
        </div>
      </div>
      {open && (
        <nav id="menu-panel" className="menu-panel" aria-label="Menu">
          {sectionLinks}
          <a href="#/book" aria-current={route.page === "book" ? "page" : undefined}>
            Lender book
          </a>
        </nav>
      )}
    </header>
  );
}
