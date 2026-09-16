import { afterEach, describe, expect, it, vi } from "vitest";

import {
  registerDiscovery,
  scanDiscovery,
  setRegistryEnabled,
  unregisterRegistry,
  updateRegistry,
} from "./api";

const response = (body: unknown) =>
  Promise.resolve(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );

afterEach(() => {
  vi.restoreAllMocks();
});

describe("registry/discovery API", () => {
  it("sends structured registry edits without a shell command field", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() =>
      response({
        id: "sample",
        name: "Sample",
        enabled: true,
        runtime: "rust",
        project_path: "sample",
        executable: "target/release/sample",
        working_dir: ".",
        args: ["--fixture"],
      }),
    );

    await updateRegistry("sample", {
      name: "Sample",
      executable: "target/release/sample",
      working_dir: ".",
      args: ["--fixture"],
    });

    const [, init] = fetchMock.mock.calls[0];
    expect(init?.method).toBe("PUT");
    expect(JSON.parse(String(init?.body))).toEqual({
      name: "Sample",
      executable: "target/release/sample",
      working_dir: ".",
      args: ["--fixture"],
    });
    expect(String(init?.body)).not.toContain("command");
  });

  it("uses explicit enable/disable and unregister mutations", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() =>
      response({
        id: "sample",
        name: "Sample",
        enabled: false,
        runtime: "rust",
        project_path: "sample",
        executable: "target/release/sample",
        working_dir: ".",
        args: [],
      }),
    );

    await setRegistryEnabled("sample", false);
    await unregisterRegistry("sample");

    expect(fetchMock.mock.calls[0][0]).toBe("/api/registry/sample/disable");
    expect(fetchMock.mock.calls[0][1]?.method).toBe("POST");
    expect(fetchMock.mock.calls[1][0]).toBe("/api/registry/sample");
    expect(fetchMock.mock.calls[1][1]?.method).toBe("DELETE");
  });

  it("keeps discovery scan and registration as separate operations", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation((input) => {
      if (String(input).endsWith("/scan")) return response([]);
      return response({
        id: "sample",
        name: "Sample",
        enabled: true,
        runtime: "rust",
        project_path: "sample",
        executable: "target/release/sample",
        working_dir: ".",
        args: [],
      });
    });

    await scanDiscovery();
    await registerDiscovery("sample", {
      id: "sample",
      name: "Sample",
      executable: "target/release/sample",
    });

    expect(fetchMock.mock.calls[0][0]).toBe("/api/discovery/scan");
    expect(fetchMock.mock.calls[0][1]?.method).toBe("POST");
    expect(fetchMock.mock.calls[1][0]).toBe("/api/discovery/sample/register");
    expect(fetchMock.mock.calls[1][1]?.method).toBe("POST");
  });
});
