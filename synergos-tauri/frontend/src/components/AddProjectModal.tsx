import { FormEvent, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Modal } from "./Modal";
import { bridgeMessage, projectOpen } from "../lib/tauri";

type Props = {
  open: boolean;
  onClose: () => void;
};

export function AddProjectModal({ open, onClose }: Props) {
  const qc = useQueryClient();
  const [projectId, setProjectId] = useState("");
  const [rootPath, setRootPath] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: async () => {
      if (!projectId.trim()) throw new Error("project ID is required");
      if (!rootPath.trim()) throw new Error("root path is required");
      await projectOpen(
        projectId.trim(),
        rootPath.trim(),
        displayName.trim() || undefined,
      );
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["projects"] });
      setProjectId("");
      setRootPath("");
      setDisplayName("");
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
      title="Add project"
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
            {mutation.isPending ? "Opening…" : "Open"}
          </button>
        </>
      }
    >
      <form onSubmit={onSubmit}>
        <div className="form-group">
          <label htmlFor="proj-id">Project ID</label>
          <input
            id="proj-id"
            value={projectId}
            onChange={(e) => setProjectId(e.target.value)}
            placeholder="myproj"
            autoFocus
          />
          <div className="help">ネットワーク全体での識別子。一意な英数字</div>
        </div>
        <div className="form-group">
          <label htmlFor="proj-path">Local path</label>
          <input
            id="proj-path"
            value={rootPath}
            onChange={(e) => setRootPath(e.target.value)}
            placeholder="D:/synergos/myproj"
          />
          <div className="help">プロジェクトルートの絶対パス</div>
        </div>
        <div className="form-group">
          <label htmlFor="proj-name">Display name (optional)</label>
          <input
            id="proj-name"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            placeholder="MyProj"
          />
        </div>
        {error ? <div className="alert error">{error}</div> : null}
      </form>
    </Modal>
  );
}
