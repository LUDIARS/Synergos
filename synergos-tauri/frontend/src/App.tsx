import { Route, Routes, NavLink } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Dashboard } from "./pages/Dashboard";
import { Settings } from "./pages/Settings";
import { daemonPing } from "./lib/tauri";

function HeaderStatus() {
  const { data: up } = useQuery({
    queryKey: ["daemon", "ping"],
    queryFn: daemonPing,
    refetchInterval: 3_000,
  });
  return (
    <div className="app-status">
      <span className={`led ${up ? "up" : "down"}`} />
      <span>{up ? "daemon up" : "daemon down"}</span>
    </div>
  );
}

export function App() {
  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand">
          <span className="dot" />
          Synergos
        </div>
        <nav>
          <NavLink to="/" end>
            Dashboard
          </NavLink>
          <NavLink to="/settings">Settings</NavLink>
        </nav>
        <HeaderStatus />
      </header>
      <main>
        <Routes>
          <Route path="/" element={<Dashboard />} />
          <Route path="/settings" element={<Settings />} />
        </Routes>
      </main>
    </div>
  );
}
