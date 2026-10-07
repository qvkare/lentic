import type { Hex } from "viem";

import { CHAIN_ID, RPC_URL, deployment } from "./chain";
import { NavBar } from "./components/NavBar";
import { short } from "./format";
import { useReducedMotion, useRoute } from "./hooks";
import { Book } from "./pages/Book";
import { Home } from "./pages/Home";
import { Station } from "./pages/Station";

/** The book and the stations read a live deployment; without one they say so and point at the simulations. */
function NoDeployment({ title }: { title: string }) {
  return (
    <main>
      <h1>{title}</h1>
      <p className="lede">
        This page reads loans from LoanBook on Base and gauges from the policy signer. Neither is connected to this
        site yet, so there is no live station to show.
      </p>
      <p className="lede">
        The simulations on the home page run the same rules in your browser: <a href="#/?s=policies">the four policies</a>{" "}
        and <a href="#/?s=try">a loan you can push to flood stage</a>.
      </p>
      {import.meta.env.DEV && (
        <p className="notice">
          Local stack: start the chain with <code>pnpm local:chain</code>, deploy with <code>pnpm local:deploy</code>,
          then reload.
        </p>
      )}
    </main>
  );
}

export function App() {
  const route = useRoute();
  const reduced = useReducedMotion();
  const d = deployment();

  return (
    <>
      <NavBar route={route} reduced={reduced} />
      <div className="shell">
        {route.page === "station" ? (
          d ? (
            <Station key={route.loanId} loanId={route.loanId as Hex} />
          ) : (
            <NoDeployment title="Station" />
          )
        ) : route.page === "book" ? (
          d ? (
            <Book />
          ) : (
            <NoDeployment title="Lender book" />
          )
        ) : (
          <Home />
        )}

        <footer className="foot">
          {d ? (
            <>
              Reading Base chain {CHAIN_ID} at {RPC_URL}. LoanBook <span className="mono">{short(d.loanBook)}</span>.
            </>
          ) : (
            <>No live deployment is connected to this site. Every scene on it is a simulation that runs in your browser.</>
          )}
        </footer>
      </div>
    </>
  );
}
