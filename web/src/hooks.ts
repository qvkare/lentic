import { useEffect, useState } from "react";

/** Polls `load` every `ms`, keeping the last good value while a refresh is in flight. */
export function usePoll<T>(load: () => Promise<T>, deps: unknown[], ms = 4000) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);

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
  }, deps);

  return { data, error };
}
