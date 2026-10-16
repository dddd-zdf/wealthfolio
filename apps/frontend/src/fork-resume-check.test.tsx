import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { usePortfolioResumeCheck } from "./use-global-event-listener";

const adapterMocks = vi.hoisted(() => ({
  isTauri: false,
  updatePortfolio: vi.fn(),
  logger: { error: vi.fn() },
}));

vi.mock("@/adapters", () => adapterMocks);

function setVisibilityState(value: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", { configurable: true, value });
  document.dispatchEvent(new Event("visibilitychange"));
}

function hideForTenMinutes() {
  setVisibilityState("hidden");
  vi.advanceTimersByTime(10 * 60 * 1000);
  setVisibilityState("visible");
}

describe("usePortfolioResumeCheck", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    adapterMocks.updatePortfolio.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("leaves updates to the server's schedule in the web app", () => {
    adapterMocks.isTauri = false;
    renderHook(() => usePortfolioResumeCheck());

    hideForTenMinutes();

    expect(adapterMocks.updatePortfolio).not.toHaveBeenCalled();
  });

  it("requests an update on resume in the desktop app", () => {
    adapterMocks.isTauri = true;
    renderHook(() => usePortfolioResumeCheck());

    hideForTenMinutes();

    expect(adapterMocks.updatePortfolio).toHaveBeenCalledTimes(1);
  });
});
