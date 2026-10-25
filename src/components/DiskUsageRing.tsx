interface DiskUsageRingProps {
  usedBytes: number;
  totalBytes: number;
  size?: number;
}

function fmt(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

export default function DiskUsageRing({
  usedBytes,
  totalBytes,
  size = 120,
}: DiskUsageRingProps) {
  const pct = totalBytes > 0 ? usedBytes / totalBytes : 0;
  const radius = 44;
  const circumference = 2 * Math.PI * radius;
  const strokeDash = pct * circumference;

  return (
    <div className="flex flex-col items-center gap-2">
      <svg width={size} height={size} viewBox="0 0 100 100">
        {/* Track */}
        <circle
          cx="50"
          cy="50"
          r={radius}
          fill="none"
          stroke="#2e3748"
          strokeWidth="10"
        />
        {/* Progress */}
        <circle
          cx="50"
          cy="50"
          r={radius}
          fill="none"
          stroke="#7c3aed"
          strokeWidth="10"
          strokeDasharray={`${strokeDash} ${circumference}`}
          strokeLinecap="round"
          transform="rotate(-90 50 50)"
        />
        {/* Center label */}
        <text
          x="50"
          y="50"
          textAnchor="middle"
          dominantBaseline="middle"
          fill="white"
          fontSize="13"
          fontWeight="600"
        >
          {(pct * 100).toFixed(0)}%
        </text>
      </svg>
      <div className="text-center">
        <p className="text-xs text-slate-400">
          {fmt(usedBytes)} used
        </p>
        <p className="text-xs text-slate-500">of {fmt(totalBytes)}</p>
      </div>
    </div>
  );
}
