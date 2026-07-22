import { beforeEach, describe, expect, it, vi } from "vitest";

// In-memory stand-in for tauri-plugin-store: `load` hands back one shared
// store whose contents persist across calls within a test.
const { data, saves } = vi.hoisted(() => ({
  data: new Map<string, unknown>(),
  saves: { count: 0 },
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  load: async () => ({
    get: async (key: string) => data.get(key),
    set: async (key: string, value: unknown) => {
      data.set(key, value);
    },
    save: async () => {
      saves.count += 1;
    },
  }),
}));

import { addRecentRepo, getRecentRepos, removeRecentRepo } from "./recents";

beforeEach(() => {
  data.clear();
  saves.count = 0;
});

describe("recents", () => {
  it("reads an empty list from a fresh store", async () => {
    expect(await getRecentRepos()).toEqual([]);
  });

  it("adds to the front and persists", async () => {
    await addRecentRepo("/repos/a");
    const list = await addRecentRepo("/repos/b");
    expect(list).toEqual(["/repos/b", "/repos/a"]);
    expect(await getRecentRepos()).toEqual(["/repos/b", "/repos/a"]);
    expect(saves.count).toBe(2);
  });

  it("re-adding an existing repo moves it to the front without duplicating", async () => {
    await addRecentRepo("/repos/a");
    await addRecentRepo("/repos/b");
    const list = await addRecentRepo("/repos/a");
    expect(list).toEqual(["/repos/a", "/repos/b"]);
  });

  it("caps the list at ten, dropping the oldest", async () => {
    for (let i = 1; i <= 11; i++) {
      await addRecentRepo(`/repos/r${i}`);
    }
    const list = await getRecentRepos();
    expect(list).toHaveLength(10);
    expect(list[0]).toBe("/repos/r11");
    expect(list).not.toContain("/repos/r1");
  });

  it("removes a repo and persists the filtered list", async () => {
    await addRecentRepo("/repos/a");
    await addRecentRepo("/repos/b");
    expect(await removeRecentRepo("/repos/a")).toEqual(["/repos/b"]);
    expect(await getRecentRepos()).toEqual(["/repos/b"]);
    // Removing something absent is a harmless no-op.
    expect(await removeRecentRepo("/repos/ghost")).toEqual(["/repos/b"]);
  });
});
