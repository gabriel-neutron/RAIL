// Small dot + label for RTL-SDR connectivity. Green = connected,
// amber = checking / idle, red = missing / disconnected. Used in the
// app header in place of free-text status.

type StatusKind = "idle" | "checking" | "found" | "missing";

export type StatusPillProps = {
  status: StatusKind;
  label: string;
  onRefresh?: () => void;
};

const KIND_CLASS: Record<StatusKind, string> = {
  idle: "status-pill-amber",
  checking: "status-pill-amber",
  found: "status-pill-green",
  missing: "status-pill-red",
};

export const StatusPill = ({ status, label, onRefresh }: StatusPillProps) => {
  return (
    <span className={`status-pill ${KIND_CLASS[status]}`}>
      <span className="status-pill-dot" aria-hidden="true" />
      <span className="status-pill-label">{label}</span>
      {onRefresh && status === "missing" && (
        <button
          type="button"
          className="status-pill-refresh"
          onClick={onRefresh}
        >
          Refresh
        </button>
      )}
    </span>
  );
};

export default StatusPill;
