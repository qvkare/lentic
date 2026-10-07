import type { Hex } from "viem";

import { CHAIN_ID, RPC_URL, deployment } from "./chain";
import { NavBar } from "./components/NavBar";
import { short } from "./format";
import { useReducedMotion, useRoute } from "./hooks";
import { Book } from "./pages/Book";
import { Home } from "./pages/Home";
import { Station } from "./pages/Station";

export function App() {
  const route = useRoute();
  const reduced = useReducedMotion();
  const d = deployment();

  return (
    <>
      <NavBar route={route} reduced={reduced} />
      <div className="shell">
        {!d ? (
          <main>
            <h1>No deployment for chain {CHAIN_ID}</h1>
            <p className="lede">
              Start the local chain with <code>pnpm local:chain</code>, deploy with <code>pnpm local:deploy</code>,
              then reload.
            </p>
          </main>
        ) : route.page === "station" ? (
          <Station key={route.loanId} loanId={route.loanId as Hex} />
        ) : route.page === "book" ? (
          <Book />
        ) : (
          <Home />
        )}

        <footer className="foot">
          Reading Base chain {CHAIN_ID} at {RPC_URL}.{" "}
          {d && (
            <>
              LoanBook <span className="mono">{short(d.loanBook)}</span>.
            </>
          )}
        </footer>
      </div>
    </>
  );
}
