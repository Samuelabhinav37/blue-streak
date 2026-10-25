interface ProgressBarProps {
  value: number; // 0–100
  label?: string;
  className?: string;
}

export default function ProgressBar({ value, label, className = "" }: ProgressBarProps) {
  const clamped = Math.max(0, Math.min(100, value));

  return (
    <div className={`w-full ${className}`}>
      {label && (
        <div className="flex justify-between text-xs text-slate-400 mb-1">
          <span>{label}</span>
          <span>{clamped.toFixed(0)}%</span>
        </div>
      )}
      <div className="h-2 bg-surface-600 rounded-full overflow-hidden">
        <div
          className="h-full bg-accent rounded-full transition-all duration-200"
          style={{ width: `${clamped}%` }}
        />
      </div>
    </div>
  );
}
