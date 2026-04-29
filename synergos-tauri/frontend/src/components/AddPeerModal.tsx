import { FormEvent, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Modal } from "./Modal";
import { bridgeMessage, peerAddUrl } from "../lib/tauri";

type Props = {
  open: boolean;
  projectId: string | null;
  onClose: () => void;
};

export function AddPeerModal({ open, projectId, onClose }: Props) {
  const qc = useQueryClient();
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: async () => {
      if (!projectId) throw new Error("no project selected");
      if (!url.trim()) throw new Error("URL is required");
      await peerAddUrl(projectId, url.trim());
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["peers"] });
      setUrl("");
      setError(null);
      onClose();
    },
    onError: (e) => setError(bridgeMessage(e)),
  });

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    mutation.mutate();
  };

  return (
    <Modal
      open={open}
      title="Add peer (URL bootstrap)"
      onClose={() => {
        setError(null);
        onClose();
      }}
      footer={
        <>
          <button onClick={onClose} disabled={mutation.isPending}>
            Cancel
          </button>
          <button
            className="primary"
            onClick={onSubmit}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? "Connecting…" : "Connect"}
          </button>
        </>
      }
    >
      <form onSubmit={onSubmit}>
        <div className="form-group">
          <label htmlFor="peer-url">Peer info URL</label>
          <input
            id="peer-url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://node1.example.com"
            autoFocus
          />
          <div className="help">
            相手 daemon の peer-info サーブレット URL
            (`/peer-info` は自動付与)
          </div>
        </div>
        {!projectId ? (
          <div className="alert info">
            プロジェクトを先に選択 / 作成してください
          </div>
        ) : null}
        {error ? <div className="alert error">{error}</div> : null}
      </form>
    </Modal>
  );
}
