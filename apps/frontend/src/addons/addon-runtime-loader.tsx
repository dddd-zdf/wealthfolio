import { useAuth } from "@/context/auth-context";
import { useEffect } from "react";
import { isSitesRuntime } from "@/features/profiles/session";
import { loadAllAddons } from "./addons-loader";

let hasStartedAddonRuntime = false;

export function AddonRuntimeLoader() {
  const { isAuthenticated, statusLoading } = useAuth();

  useEffect(() => {
    if (isSitesRuntime || statusLoading || !isAuthenticated || hasStartedAddonRuntime) {
      return;
    }

    hasStartedAddonRuntime = true;
    void loadAllAddons();
  }, [statusLoading, isAuthenticated]);

  return null;
}
