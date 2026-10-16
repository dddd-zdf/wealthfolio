import { isSameDay } from "date-fns";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/button";
import type { DateRange } from "./date-range-selector";

interface YearPresetsProps {
  /** How many calendar years to offer, current year to date first. */
  count: number;
  asOf: Date;
  selected: DateRange | undefined;
  onPick: (range: DateRange) => void;
}

const sameDay = (date1: Date | undefined, date2: Date | undefined) => !!date1 && !!date2 && isSameDay(date1, date2);

/** Fork: quick picks for whole calendar years in the custom range picker. */
export function YearPresets({ count, asOf, selected, onPick }: YearPresetsProps) {
  const { t } = useTranslation();
  const currentYear = asOf.getFullYear();
  const calendarYearRange = (year: number): DateRange => ({
    from: new Date(year, 0, 1),
    to: year === currentYear ? asOf : new Date(year, 11, 31),
  });
  return (
    <div
      className="flex flex-wrap gap-1.5"
      role="group"
      aria-label={t("fork:dateRange.calendarYears", "Calendar years")}
    >
      {Array.from({ length: count }, (_, index) => currentYear - index).map((year) => {
        const range = calendarYearRange(year);
        const isSelected = sameDay(selected?.from, range.from) && sameDay(selected?.to, range.to);
        return (
          <Button
            key={year}
            type="button"
            size="sm"
            variant={isSelected ? "default" : "outline"}
            className="h-7 rounded-full px-3 text-xs"
            onClick={() => onPick(range)}
          >
            {year}
          </Button>
        );
      })}
    </div>
  );
}
