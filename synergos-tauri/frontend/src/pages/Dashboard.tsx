import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  bridgeMessage,
  daemonStatus,
  peerList,
  projectList,
  transferList,
  TransferInfo,
} from "../lib/tauri";
import { AddProjectModal } from "../components/AddProjectModal";
import { AddPeerModal } from "../components/AddPeerModal";
import { PublishModal } from "../components/PublishModal";

function ProjectsSection({
  selectedId,
  onSelect,
  onAddOpen,
  onPublishOpen,
}: {
  selectedId: string | null;
  onSelect: (id: string) => void;
  onAddOpen: () => void;
  onPublishOpen: (projectId: string, rootPath: string) => void;
}) {
  const { data, isLoading, error } = useQuery({
    queryKey: ["projects"],
    queryFn: projectList,
    refetchInterval: 5_000,
  });
  return (
    <section className="section">
      <div className="section-header">
        <h2>Projects</h2>
        <div className="toolbar">
          <button className="primary" onClick={onAddOpen}>
            + Add project
          </button>
        </div>
      </div>
      <div className="section-body">
        {isLoading ? (
          <div className="list-empty">Loading…</div>
        ) : error ? (
          <div className="alert error">{bridgeMessage(error)}</div>
        ) : !data || data.length === 0 ? (
          <div className="list-empty">プロジェクトがありません</div>
        ) : (
          <div className="list">
            {data.map((p) => (
              <div
                key={p.project_id}
                className="row"
                style={{
                  borderColor:
                    selectedId === p.project_id ? "var(--accent)" : undefined,
                  cursor: "pointer",
                }}
                onClick={() => onSelect(p.project_id)}
              >
                <div className="meta">
                  <div className="title">{p.display_name}</div>
                  <div className="sub">
                    {p.project_id} — {p.root_path}
                  </div>
                </div>
                <div className="actions">
                  <span className="badge blue">{p.peer_count} peers</span>
                  {p.active_transfers > 0 ? (
                    <span className="badge orange">
                      {p.active_transfers} txfr
                    </span>
                  ) : null}
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onPublishOpen(p.project_id, p.root_path);
                    }}
                    style={{ padding: "0.25rem 0.6rem", fontSize: "0.75rem" }}
                  >
                    Publish…
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

function TransfersSection({ projectId }: { projectId: string | null }) {
  const { data, isLoading, error } = useQuery({
    queryKey: ["transfers", projectId],
    queryFn: () => transferList(projectId ?? undefined),
    refetchInterval: 3_000,
  });
  return (
    <section className="section">
      <div className="section-header">
        <h2>
          Transfers
          {projectId ? (
            <span className="badge gray" style={{ marginLeft: "0.5rem" }}>
              {projectId}
            </span>
          ) : (
            <span className="badge gray" style={{ marginLeft: "0.5rem" }}>
              all
            </span>
          )}
        </h2>
      </div>
      <div className="section-body">
        {isLoading ? (
          <div className="list-empty">Loading…</div>
        ) : error ? (
          <div className="alert error">{bridgeMessage(error)}</div>
        ) : !data || data.length === 0 ? (
          <div className="list-empty">アクティブ転送なし</div>
        ) : (
          <div className="list">
            {data.map((t) => (
              <TransferRow key={t.transfer_id} t={t} />
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

function TransferRow({ t }: { t: TransferInfo }) {
  const pct =
    t.file_size > 0
      ? Math.min(100, Math.floor((t.bytes_transferred / t.file_size) * 100))
      : 0;
  return (
    <div className="row">
      <div className="meta">
        <div className="title">
          <span style={{ marginRight: "0.5rem" }}>
            {t.direction === "Send" ? "↑" : "↓"}
          </span>
          {t.file_name}
        </div>
        <div className="sub">
          {pct}% · {formatBps(t.speed_bps)} · {formatBytes(t.bytes_transferred)}
          /{formatBytes(t.file_size)}
        </div>
      </div>
      <div className="actions">
        <span className={`badge ${transferStateBadge(t.state)}`}>{t.state}</span>
      </div>
    </div>
  );
}

function transferStateBadge(state: string): string {
  const s = state.toLowerCase();
  if (s.includes("complet") || s === "done") return "green";
  if (s.includes("fail") || s.includes("cancel")) return "red";
  if (s.includes("transfer") || s.includes("progress")) return "orange";
  return "gray";
}

function formatBytes(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)} GB`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)} MB`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)} KB`;
  return `${n} B`;
}

function PeersSection({
  projectId,
  onAddOpen,
}: {
  projectId: string | null;
  onAddOpen: () => void;
}) {
  const { data, isLoading, error } = useQuery({
    queryKey: ["peers", projectId],
    queryFn: () => peerList(projectId!),
    enabled: !!projectId,
    refetchInterval: projectId ? 5_000 : false,
  });
  return (
    <section className="section">
      <div className="section-header">
        <h2>
          Peers
          {projectId ? (
            <span className="badge gray" style={{ marginLeft: "0.5rem" }}>
              {projectId}
            </span>
          ) : null}
        </h2>
        <div className="toolbar">
          <button
            className="primary"
            onClick={onAddOpen}
            disabled={!projectId}
          >
            + Add peer
          </button>
        </div>
      </div>
      <div className="section-body">
        {!projectId ? (
          <div className="list-empty">
            プロジェクトを選択すると参加中の peer が表示されます
          </div>
        ) : isLoading ? (
          <div className="list-empty">Loading…</div>
        ) : error ? (
          <div className="alert error">{bridgeMessage(error)}</div>
        ) : !data || data.length === 0 ? (
          <div className="list-empty">
            まだ peer がいません。`Add peer` で URL bootstrap してください
          </div>
        ) : (
          <div className="list">
            {data.map((p) => (
              <div key={p.peer_id} className="row">
                <div className="meta">
                  <div className="title">
                    {p.display_name || p.peer_id.slice(0, 12) + "…"}
                    {p.synergos_version ? (
                      <span
                        className="badge gray"
                        style={{ marginLeft: "0.5rem", fontSize: "0.65rem" }}
                      >
                        v{p.synergos_version}
                      </span>
                    ) : null}
                  </div>
                  <div className="sub">
                    {p.route} · {p.rtt_ms} ms · {formatBps(p.bandwidth_bps)}
                  </div>
                </div>
                <div className="actions">
                  <span className={`badge ${stateBadge(p.state)}`}>
                    {p.state}
                  </span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

function StatusSection() {
  const { data, error } = useQuery({
    queryKey: ["daemon", "status"],
    queryFn: daemonStatus,
    refetchInterval: 5_000,
  });
  if (error) {
    return (
      <div className="banner">
        Daemon に接続できません: {bridgeMessage(error)}。`synergos-core start`
        が起動しているか、Settings → Daemon paths を確認してください。
      </div>
    );
  }
  if (!data) return null;
  return (
    <div className="kv" style={{ marginBottom: "0.75rem" }}>
      <div className="k">PID</div>
      <div>{data.pid}</div>
      <div className="k">Projects</div>
      <div>{data.project_count}</div>
      <div className="k">Connections</div>
      <div>{data.active_connections}</div>
      <div className="k">Transfers</div>
      <div>{data.active_transfers}</div>
    </div>
  );
}

export function Dashboard() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [addProject, setAddProject] = useState(false);
  const [addPeer, setAddPeer] = useState(false);
  const [publishCtx, setPublishCtx] = useState<{
    projectId: string;
    rootPath: string;
  } | null>(null);

  return (
    <>
      <StatusSection />
      <ProjectsSection
        selectedId={selectedId}
        onSelect={setSelectedId}
        onAddOpen={() => setAddProject(true)}
        onPublishOpen={(projectId, rootPath) =>
          setPublishCtx({ projectId, rootPath })
        }
      />
      <PeersSection
        projectId={selectedId}
        onAddOpen={() => setAddPeer(true)}
      />
      <TransfersSection projectId={selectedId} />
      <AddProjectModal
        open={addProject}
        onClose={() => setAddProject(false)}
      />
      <AddPeerModal
        open={addPeer}
        projectId={selectedId}
        onClose={() => setAddPeer(false)}
      />
      <PublishModal
        open={!!publishCtx}
        projectId={publishCtx?.projectId ?? null}
        rootPath={publishCtx?.rootPath ?? null}
        onClose={() => setPublishCtx(null)}
      />
    </>
  );
}

function formatBps(bps: number): string {
  if (bps >= 1_000_000) return `${(bps / 1_000_000).toFixed(1)} Mbps`;
  if (bps >= 1_000) return `${(bps / 1_000).toFixed(1)} kbps`;
  return `${bps} bps`;
}

function stateBadge(state: string): string {
  const s = state.toLowerCase();
  if (s.includes("connect")) return "green";
  if (s.includes("disconnect") || s.includes("expir")) return "red";
  if (s.includes("connecting") || s.includes("pending")) return "orange";
  return "gray";
}
