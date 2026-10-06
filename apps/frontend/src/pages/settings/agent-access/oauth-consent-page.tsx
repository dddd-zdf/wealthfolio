import { decideMcpOAuth } from "@/adapters";
import { cn } from "@/lib/utils";
import {
  ApplicationShell,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@wealthfolio/ui";
import { Icons } from "@wealthfolio/ui/components/ui/icons";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";
import {
  applyScopeDependencies,
  READ_SCOPES,
  scopeDescription,
  scopeLabel,
  SCOPES,
  type ScopeKey,
} from "./scopes";

/**
 * Consent screen for an MCP client (Claude, ChatGPT, ...) connecting over
 * OAuth. The server's `/oauth/authorize` validates the request and sends the
 * browser here; the app's login gate runs first.
 */
export default function OAuthConsentPage() {
  const { t } = useTranslation();
  const [params] = useSearchParams();
  const clientId = params.get("client_id");
  const clientName = params.get("client_name") || "MCP client";
  const redirectUri = params.get("redirect_uri");
  const codeChallenge = params.get("code_challenge");
  const state = params.get("state") ?? undefined;

  const [selected, setSelected] = useState<Set<ScopeKey>>(() => {
    const known = new Set<string>(SCOPES.map((scope) => scope.key));
    const requested = (params.get("scope") ?? "").split(" ").filter((s) => known.has(s));
    return new Set(applyScopeDependencies(requested.length > 0 ? requested : READ_SCOPES));
  });
  const scopes = useMemo(() => applyScopeDependencies(selected), [selected]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  let redirectHost: string | null = null;
  try {
    redirectHost = redirectUri ? new URL(redirectUri).host : null;
  } catch {
    redirectHost = null;
  }
  const valid = Boolean(clientId && redirectHost && codeChallenge);

  const toggle = (key: ScopeKey) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const decide = async (approve: boolean) => {
    if (!clientId || !redirectUri || !codeChallenge) return;
    setPending(true);
    setError(null);
    try {
      const url = await decideMcpOAuth({
        clientId,
        redirectUri,
        codeChallenge,
        state,
        scopes: approve ? scopes : undefined,
      });
      window.location.assign(url);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setPending(false);
    }
  };

  const renderGroup = (group: "read" | "write", heading: string) => (
    <div className="space-y-2">
      <p className="text-muted-foreground text-xs font-medium uppercase">{heading}</p>
      {SCOPES.filter((scope) => scope.group === group).map((scope) => {
        const checked = scopes.includes(scope.key);
        return (
          <button
            key={scope.key}
            type="button"
            role="checkbox"
            aria-checked={checked}
            title={scopeDescription(t, scope.key)}
            disabled={pending}
            onClick={() => toggle(scope.key)}
            className={cn(
              "flex w-full items-center justify-between gap-2 rounded-md border px-3 py-2 text-left text-sm transition-colors",
              checked ? "border-success/50 bg-success/10" : "border-border hover:bg-muted/40",
            )}
          >
            <span className="truncate font-medium">{scopeLabel(t, scope.key)}</span>
            {checked && <Icons.Check className="text-success h-4 w-4 shrink-0" />}
          </button>
        );
      })}
    </div>
  );

  return (
    <ApplicationShell className="fixed inset-0 flex items-center justify-center overflow-y-auto p-6">
      <Card className="my-auto w-full max-w-lg">
        <CardHeader>
          <CardTitle>{t("settings:agentAccess.oauth_title", { client: clientName })}</CardTitle>
          <CardDescription>
            {valid
              ? t("settings:agentAccess.oauth_description", {
                  client: clientName,
                  host: redirectHost,
                })
              : t("settings:agentAccess.oauth_invalid")}
          </CardDescription>
        </CardHeader>
        {valid && (
          <>
            <CardContent className="space-y-4">
              {renderGroup("read", t("settings:agentAccess.dialog_read_access"))}
              {renderGroup("write", t("settings:agentAccess.dialog_write_access"))}
              <p className="text-muted-foreground text-xs">
                {t("settings:agentAccess.oauth_revoke_hint")}
              </p>
              {error && (
                <p className="text-destructive text-sm" role="alert">
                  {error}
                </p>
              )}
            </CardContent>
            <CardFooter className="flex justify-end gap-2">
              <Button variant="outline" disabled={pending} onClick={() => decide(false)}>
                {t("settings:agentAccess.oauth_deny")}
              </Button>
              <Button disabled={pending || scopes.length === 0} onClick={() => decide(true)}>
                {t("settings:agentAccess.oauth_allow")}
              </Button>
            </CardFooter>
          </>
        )}
      </Card>
    </ApplicationShell>
  );
}
