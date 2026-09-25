import { ProfileShell } from "@/features/profiles/profile-shell";
import { ProfileContext } from "@/features/profiles/profile-context";
import type { ProfileSummary } from "@/features/profiles/api";
import { isSitesRuntime } from "@/features/profiles/session";
import { NativeDatabaseGate } from "@/features/database-recovery/native-database-gate";
import { isWeb } from "@/adapters";
import { AddonRuntimeLoader } from "@/addons/addon-runtime-loader";
import { setAddonQueryClient } from "@/addons/addons-runtime-context";
import { AssetLogoRegistrySync } from "@/components/asset-logo-registry-sync";
import { Toaster } from "@/components/sonner";
import { AuthGate, AuthProvider } from "@/context/auth-context";
import { EventDialogProvider } from "@/features/spending/components/event-dialog-provider";
import { WealthfolioConnectProvider } from "@/features/wealthfolio-connect";
import { SettingsProvider } from "@/lib/settings-provider";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@wealthfolio/ui";
import { useEffect, useState, type ReactNode } from "react";
import { PrivacyProvider } from "./context/privacy-context";
import { LoginPage } from "./pages/auth/login-page";
import { AppRoutes } from "./routes";
import { registerSitesMcpTools } from "@/features/sites-mcp";

const SITES_PROFILE: ProfileSummary = {
  id: "sites-owner",
  name: "Private Site",
  avatarId: "clay-pebble-animated",
  lockEnabled: false,
};

function SitesProfileProvider({ children }: { children: ReactNode }) {
  return (
    <ProfileContext.Provider
      value={{
        profile: SITES_PROFILE,
        profileCount: 1,
        profiles: [SITES_PROFILE],
        addProfile: () => {},
        manageProfile: () => {},
        lockProfile: () => {},
        switchProfile: () => {},
        selectProfile: () => {},
      }}
    >
      {children}
    </ProfileContext.Provider>
  );
}

function SitesMcpRegistrar() {
  useEffect(() => {
    if (!isSitesRuntime) return;
    let cancelled = false;
    let unregister: (() => void) | undefined;
    void registerSitesMcpTools()
      .then((registration) => {
        if (cancelled) registration?.unregister();
        else unregister = registration?.unregister;
      })
      .catch(() => {
        // WebMCP support varies by host; the app remains usable without it.
      });
    return () => {
      cancelled = true;
      unregister?.();
    };
  }, []);
  return null;
}

function App() {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            refetchOnWindowFocus: false,
            staleTime: 5 * 60 * 1000,
            retry: false,
          },
        },
      }),
  );

  const isWebEnv = isWeb;
  const sitesMode = isSitesRuntime;

  setAddonQueryClient(queryClient as unknown as Parameters<typeof setAddonQueryClient>[0]);

  const content = (
    <SettingsProvider>
      <WealthfolioConnectProvider>
        <PrivacyProvider>
          <TooltipProvider>
            <Toaster mobileOffset={{ top: "68px" }} closeButton expand={false} />
            <AddonRuntimeLoader />
            <SitesMcpRegistrar />
            <EventDialogProvider>
              <AssetLogoRegistrySync />
              <AppRoutes />
            </EventDialogProvider>
          </TooltipProvider>
        </PrivacyProvider>
      </WealthfolioConnectProvider>
    </SettingsProvider>
  );

  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        {sitesMode ? (
          <SitesProfileProvider>
            <NativeDatabaseGate>{content}</NativeDatabaseGate>
          </SitesProfileProvider>
        ) : isWebEnv ? (
          <AuthGate fallback={<LoginPage />}>
            <ProfileShell>
              <NativeDatabaseGate>{content}</NativeDatabaseGate>
            </ProfileShell>
          </AuthGate>
        ) : (
          <ProfileShell>
            <NativeDatabaseGate>{content}</NativeDatabaseGate>
          </ProfileShell>
        )}
      </AuthProvider>
    </QueryClientProvider>
  );
}

export default App;
