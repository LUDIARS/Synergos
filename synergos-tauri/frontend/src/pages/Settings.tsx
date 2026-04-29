import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  bridgeMessage,
  configGet,
  configPaths,
  configSet,
  ConfigFile,
} from "../lib/tauri";

export function Settings() {
  const qc = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ["config"],
    queryFn: configGet,
  });
  const { data: paths } = useQuery({
    queryKey: ["config", "paths"],
    queryFn: configPaths,
  });

  const [draft, setDraft] = useState<ConfigFile | null>(null);
  const [bootstrapText, setBootstrapText] = useState("");
  const [saved, setSaved] = useState(false);
  const [saveErr, setSaveErr] = useState<string | null>(null);

  useEffect(() => {
    if (data) {
      setDraft(data);
      setBootstrapText(data.bootstrap_urls.join("\n"));
    }
  }, [data]);

  const mutation = useMutation({
    mutationFn: (cfg: ConfigFile) => configSet(cfg),
    onSuccess: () => {
      setSaved(true);
      setSaveErr(null);
      qc.invalidateQueries({ queryKey: ["config"] });
      setTimeout(() => setSaved(false), 2_500);
    },
    onError: (e) => {
      setSaveErr(bridgeMessage(e));
      setSaved(false);
    },
  });

  if (isLoading || !draft) {
    return <div className="list-empty">Loading config…</div>;
  }
  if (error) {
    return <div className="alert error">{bridgeMessage(error)}</div>;
  }

  const onSave = () => {
    const next: ConfigFile = {
      ...draft,
      bootstrap_urls: bootstrapText
        .split("\n")
        .map((s) => s.trim())
        .filter((s) => s.length > 0),
    };
    setDraft(next);
    mutation.mutate(next);
  };

  return (
    <>
      <section className="section">
        <div className="section-header">
          <h2>Connection</h2>
          <div className="toolbar">
            <button
              className="primary"
              onClick={onSave}
              disabled={mutation.isPending}
            >
              {mutation.isPending ? "Saving…" : "Save"}
            </button>
          </div>
        </div>
        <div className="section-body">
          <div className="form-group">
            <label htmlFor="bootstrap-urls">Bootstrap URLs</label>
            <textarea
              id="bootstrap-urls"
              rows={3}
              value={bootstrapText}
              onChange={(e) => setBootstrapText(e.target.value)}
              placeholder="https://node1.example.com"
              style={{ resize: "vertical" }}
            />
            <div className="help">
              起動時に自動で peer-info を GET → QUIC connect します。1 行 1 URL
            </div>
          </div>
          <div className="form-group">
            <label>
              <input
                type="checkbox"
                checked={draft.force_relay_only}
                onChange={(e) =>
                  setDraft({ ...draft, force_relay_only: e.target.checked })
                }
                style={{ width: "auto", marginRight: "0.5rem" }}
              />
              Force relay-only
            </label>
            <div className="help">
              全 peer 通信を WebSocket relay 経由に強制 (UDP/IPv6 直結を諦める)
            </div>
          </div>
          <div className="form-group">
            <label>
              <input
                type="checkbox"
                checked={draft.auto_promote}
                onChange={(e) =>
                  setDraft({ ...draft, auto_promote: e.target.checked })
                }
                style={{ width: "auto", marginRight: "0.5rem" }}
              />
              Auto promote (NAT 越え probe)
            </label>
            <div className="help">
              起動時に IPv6 / UPnP / Tunnel 到達性を自動診断
            </div>
          </div>
          <div className="form-group">
            <label htmlFor="quic-listen">QUIC listen address</label>
            <input
              id="quic-listen"
              value={draft.quic_listen_addr ?? ""}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  quic_listen_addr: e.target.value || null,
                })
              }
              placeholder="[::]:7777"
            />
            <div className="help">
              空ならデフォルト (`[::]:0`、カーネル割当)
            </div>
          </div>
          <div className="form-group">
            <label htmlFor="peer-info-listen">peer-info servlet listen address</label>
            <input
              id="peer-info-listen"
              value={draft.peer_info_listen_addr ?? ""}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  peer_info_listen_addr: e.target.value || null,
                })
              }
              placeholder="127.0.0.1:7780"
            />
            <div className="help">
              公開ノード用。空なら servlet を起動しない
            </div>
          </div>
          {saved ? (
            <div className="alert info">
              保存しました。daemon を再起動すると反映されます。
            </div>
          ) : null}
          {saveErr ? <div className="alert error">{saveErr}</div> : null}
        </div>
      </section>
      {paths ? (
        <section className="section">
          <div className="section-header">
            <h2>Daemon paths</h2>
          </div>
          <div className="section-body">
            <div className="kv">
              <div className="k">Config</div>
              <div>{paths.config_path}</div>
              <div className="k">Identity</div>
              <div>{paths.identity_path}</div>
              <div className="k">Projects</div>
              <div>{paths.projects_path}</div>
            </div>
          </div>
        </section>
      ) : null}
    </>
  );
}
