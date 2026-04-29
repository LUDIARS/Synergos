import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Modal } from "./Modal";
import { bridgeMessage, projectPublish } from "../lib/tauri";

type Props = {
  open: boolean;
  projectId: string | null;
  rootPath: string | null;
  onClose: () => void;
};

export function PublishModal({ open, projectId, rootPath, onClose }: Props) {
  const qc = useQueryClient();
  const [files, setFiles] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  const browse = async () => {
    setError(null);
    if (!rootPath) return;
    try {
      const picked = await openDialog({
        multiple: true,
        directory: false,
        defaultPath: rootPath,
        title: "Select files to publish (within project root)",
      });
      if (!picked) return;
      const list = Array.isArray(picked) ? picked : [picked];
      // path はプロジェクトルート相対 or 絶対のどちらでも daemon 側で正規化される
      setFiles(list);
    } catch (e) {
      setError(bridgeMessage(e));
    }
  };

  const removeAt = (idx: number) => {
    setFiles((prev) => prev.filter((_, i) => i !== idx));
  };

  const mutation = useMutation({
    mutationFn: async () => {
      if (!projectId) throw new Error("no project selected");
      if (files.length === 0) throw new Error("no files selected");
      await projectPublish(projectId, files);
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["transfers"] });
      qc.invalidateQueries({ queryKey: ["projects"] });
      setFiles([]);
      setError(null);
      onClose();
    },
    onError: (e) => setError(bridgeMessage(e)),
  });

  return (
    <Modal
      open={open}
      title="Publish files"
      onClose={() => {
        setError(null);
        setFiles([]);
        onClose();
      }}
      footer={
        <>
          <button onClick={onClose} disabled={mutation.isPending}>
            Cancel
          </button>
          <button onClick={browse} disabled={mutation.isPending || !rootPath}>
            Browse files…
          </button>
          <button
            className="primary"
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending || files.length === 0}
          >
            {mutation.isPending
              ? "Publishing…"
              : `Publish ${files.length || ""}`.trim()}
          </button>
        </>
      }
    >
      <div className="form-group">
        <label>Project</label>
        <input value={projectId ?? ""} disabled readOnly />
      </div>
      <div className="form-group">
        <label>Root path</label>
        <input value={rootPath ?? ""} disabled readOnly />
        <div className="help">
          選択するファイルはこのプロジェクトルート配下にある必要があります
          (daemon が path traversal を拒否します)
        </div>
      </div>
      <div className="form-group">
        <label>Selected files ({files.length})</label>
        {files.length === 0 ? (
          <div className="list-empty" style={{ padding: "0.75rem" }}>
            "Browse files…" でファイルを選択
          </div>
        ) : (
          <div className="list">
            {files.map((f, i) => (
              <div key={`${f}-${i}`} className="row">
                <div className="meta">
                  <div
                    className="title"
                    style={{ fontFamily: "monospace", fontSize: "0.8rem" }}
                  >
                    {f}
                  </div>
                </div>
                <div className="actions">
                  <button
                    className="danger"
                    onClick={() => removeAt(i)}
                    disabled={mutation.isPending}
                    style={{ padding: "0.2rem 0.5rem", fontSize: "0.75rem" }}
                  >
                    Remove
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
      {error ? <div className="alert error">{error}</div> : null}
    </Modal>
  );
}
