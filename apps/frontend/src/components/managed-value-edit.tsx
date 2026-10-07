import { logger, updateQuote } from "@/adapters";
import { managedPrices } from "@/lib/managed-account";
import { invalidatePerformanceCaches } from "@/lib/performance-cache";
import { QueryKeys } from "@/lib/query-keys";
import type { Holding } from "@/lib/types";
import { generateTempId, toQuote } from "@/pages/asset/quote-history-utils";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Icons, MoneyInput, PrivacyAmount } from "@wealthfolio/ui";
import { toast } from "@wealthfolio/ui/components/ui/use-toast";
import { useState } from "react";
import { useTranslation } from "react-i18next";

interface ManagedValueEditProps {
  /** The account's manually priced holdings (see `managedHoldings`). */
  holdings: Holding[];
  /** The account's total value, in `currency`. */
  value: number;
  currency: string;
}

/**
 * An account total with a pencil. Typing a new total saves today's prices for
 * the account's manually priced holdings so the account adds up to it.
 */
export function ManagedValueEdit({ holdings, value, currency }: ManagedValueEditProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [isEditing, setIsEditing] = useState(false);
  const [draft, setDraft] = useState<number | undefined>();

  const mutation = useMutation({
    mutationFn: async (newTotal: number) => {
      const prices = managedPrices(holdings, value, newTotal);
      if (!prices) throw new Error("Value too low for the account's other holdings");
      const date = new Date();
      for (const { assetId, price, currency: quoteCurrency } of prices) {
        const quote = toQuote(
          {
            id: generateTempId(),
            date,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 0,
            currency: quoteCurrency,
          },
          assetId,
        );
        await updateQuote(assetId, quote);
      }
      return prices;
    },
    onSuccess: async (prices) => {
      setIsEditing(false);
      toast({ title: t("asset:updateValuation.update_value"), variant: "success" });
      invalidatePerformanceCaches(queryClient);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: [QueryKeys.LATEST_QUOTES] }),
        ...prices.flatMap(({ assetId }) => [
          queryClient.invalidateQueries({ queryKey: [QueryKeys.ASSET_DATA, assetId] }),
          queryClient.invalidateQueries({ queryKey: [QueryKeys.QUOTE_HISTORY, assetId] }),
        ]),
      ]);
    },
    onError: (error) => {
      logger.error(`Error updating account value: ${error}`);
      toast({
        title: "Uh oh! Something went wrong.",
        description: "There was a problem updating the value.",
        variant: "destructive",
      });
    },
  });

  const save = () => {
    if (draft != null && draft > 0) mutation.mutate(draft);
  };

  // Dashboard rows are links; keep clicks here from navigating.
  const stop = (event: React.SyntheticEvent) => {
    event.preventDefault();
    event.stopPropagation();
  };

  if (isEditing) {
    return (
      <span className="flex items-center gap-1.5" onClick={stop}>
        <MoneyInput
          autoFocus
          aria-label={t("asset:updateValuation.new_value")}
          className="h-8 w-36 text-sm"
          value={draft}
          onValueChange={setDraft}
          onKeyDown={(event) => {
            if (event.key === "Enter") save();
            if (event.key === "Escape") setIsEditing(false);
          }}
        />
        <Button
          size="icon"
          className="size-8 shrink-0"
          onClick={save}
          disabled={mutation.isPending}
          aria-label={t("asset:updateValuation.update_value")}
        >
          {mutation.isPending ? (
            <Icons.Spinner className="size-4 animate-spin" />
          ) : (
            <Icons.Check className="size-4" />
          )}
        </Button>
        <Button
          size="icon"
          variant="outline"
          className="size-8 shrink-0"
          onClick={() => setIsEditing(false)}
          aria-label={t("common:cancel")}
        >
          <Icons.Close className="size-4" />
        </Button>
      </span>
    );
  }

  return (
    <span className="inline-flex items-center gap-1.5">
      <PrivacyAmount value={value} currency={currency} />
      <button
        type="button"
        className="text-muted-foreground hover:text-foreground"
        aria-label={t("asset:updateValuation.update_value")}
        onClick={(event) => {
          stop(event);
          setDraft(Number(value.toFixed(2)));
          setIsEditing(true);
        }}
      >
        <Icons.Pencil className="size-3.5" />
      </button>
    </span>
  );
}
