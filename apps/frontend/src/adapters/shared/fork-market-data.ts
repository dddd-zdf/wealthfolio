// Fork: market data calls kept out of upstream's market-data.ts.
import type { Quote } from "@/lib/types";

import { invoke } from "./platform";

/** One asset's live price bars for the last session ("1D") or five ("1W"). */
export const getAssetIntradayQuotes = async (
  assetId: string,
  range: "1D" | "1W",
): Promise<Quote[]> => {
  return invoke<Quote[]>("get_asset_intraday_quotes", { assetId, range });
};
