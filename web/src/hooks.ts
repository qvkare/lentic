import { useCallback, useEffect, useState } from "react";

/**
 * Polls `load` every `ms`, keeping the last good value while a refresh is in flight.
 * `data` is undefined until the first answer, so "loading" never reads as "not found".
 */
export function usePoll<T>(load: () => Promise<T>, deps: unknown[], ms = 5000) {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let alive = true;
    const run = () =>
      load()
        .then((v) => {
          if (!alive) return;
          setData(v);
          setError(null);
        })
        .catch((e: unknown) => alive && setError(e instanceof Error ? e.message : String(e)));
    run();
    const id = setInterval(run, ms);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [...deps, tick]);

  return { data, error, refresh: () => setTick((t) => t + 1) };
}

export type Route = { page: "home" } | { page: "book" } | { page: "station"; loanId: string };

/** Hash route: "#/" home, "#/book" the lender book, "#/station/<loanId>" a station. */
export function useRoute(): Route {
  const read = (): Route => {
    const hash = window.location.hash;
    const m = hash.match(/^#\/station\/(0x[0-9a-fA-F]{64})$/);
    if (m) return { page: "station", loanId: m[1] };
    return hash === "#/book" ? { page: "book" } : { page: "home" };
  };
  const [route, setRoute] = useState(read);
  useEffect(() => {
    const on = () => setRoute(read());
    window.addEventListener("hashchange", on);
    return () => window.removeEventListener("hashchange", on);
  }, []);
  return route;
}

/** Width of an element, for SVG drawn at real pixel size. Works when the element mounts late. */
export function useWidth<T extends HTMLElement>() {
  const [el, setEl] = useState<T | null>(null);
  const [width, setWidth] = useState(0);
  const ref = useCallback((node: T | null) => setEl(node), []);
  useEffect(() => {
    if (!el) return;
    // Measure now: ResizeObserver only reports on a rendering frame, which a hidden tab or a
    // headless capture may not produce before the element is needed.
    setWidth(Math.round(el.getBoundingClientRect().width));
    const ro = new ResizeObserver(([entry]) => setWidth(Math.round(entry.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return { ref, width };
}

/** Follows the visitor's reduced-motion preference, live. */
export function useReducedMotion(): boolean {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduced, setReduced] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const on = () => setReduced(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return reduced;
}
