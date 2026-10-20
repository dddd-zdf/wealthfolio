// Fork: asset price chart for 1D/1W live intraday bars (see
// fork-asset-intraday.ts). Kept apart from upstream's history-chart-symbol so
// the upstream chart stays unchanged; styled to match it.
import { formatDate, formatTime } from "@/lib/utils";
import { useAmountFormatting, useDateFormatting } from "@wealthfolio/ui";
import {
  Area,
  AreaChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { AssetIntradayChart, AssetIntradayPoint } from "./fork-asset-intraday";

const timeOf = (point: AssetIntradayPoint) => Date.parse(point.timestamp);

interface TooltipProps {
  active?: boolean;
  payload?: { value: number; payload: AssetIntradayPoint }[];
}

export function ForkIntradayPriceChart({
  chart,
  height = 350,
}: {
  chart: AssetIntradayChart;
  height?: number;
}) {
  const { timeDomain, referenceValue } = chart;
  return (
    <div className="relative flex h-full flex-col" data-no-swipe-drag>
      <div className="grow">
        <ResponsiveContainer width="100%" height="100%" minHeight={height}>
          <AreaChart data={chart.points} margin={{ top: 0, right: 0, left: 0, bottom: 0 }}>
            <defs>
              <linearGradient id="forkIntradayFill" x1="0" y1="0" x2="0" y2="1">
                <stop offset="5%" stopColor="var(--success)" stopOpacity={0.2} />
                <stop offset="95%" stopColor="var(--success)" stopOpacity={0.1} />
              </linearGradient>
            </defs>
            <Tooltip
              content={(props) => <IntradayTooltip {...(props as unknown as TooltipProps)} />}
              wrapperStyle={{ pointerEvents: "none" }}
            />
            <YAxis hide type="number" domain={["auto", "auto"]} />
            {/* One XAxis element: swapping two (1D <-> 1W) leaves Recharts on the old axis. */}
            <XAxis
              hide
              dataKey={timeDomain ? timeOf : "timestamp"}
              type={timeDomain ? "number" : "category"}
              scale={timeDomain ? "time" : "auto"}
              domain={timeDomain}
            />
            {referenceValue !== undefined && (
              <ReferenceLine
                y={referenceValue}
                stroke="var(--muted-foreground)"
                strokeDasharray="2 4"
                strokeOpacity={0.4}
                ifOverflow="extendDomain"
              />
            )}
            <Area
              isAnimationActive={true}
              animationDuration={300}
              animationEasing="ease-out"
              connectNulls={true}
              type="monotone"
              dataKey="totalValue"
              stroke="var(--success)"
              fillOpacity={1}
              fill="url(#forkIntradayFill)"
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}

function IntradayTooltip({ active, payload }: TooltipProps) {
  const amountFormatting = useAmountFormatting();
  const dateFormatting = useDateFormatting();
  if (!active || !payload?.length) {
    return null;
  }
  const point = payload[0].payload;
  return (
    <div className="bg-popover pointer-events-none grid grid-cols-1 gap-1.5 rounded-md border p-2 shadow-md">
      <p className="text-muted-foreground text-xs">
        {formatDate(point.timestamp, dateFormatting)} {formatTime(point.timestamp, dateFormatting)}
      </p>
      <p className="text-base font-bold">
        {amountFormatting.formatPrice(payload[0].value, point.currency, false)}
      </p>
    </div>
  );
}
