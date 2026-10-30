// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { validateProfile } from "./command-plan.mjs";

export class ProfileStore {
  constructor(filename) { this.filename = filename; }
  list() {
    let contents;
    try { contents = readFileSync(this.filename, "utf8"); }
    catch (error) { if (error.code === "ENOENT") return []; throw error; }
    if (Buffer.byteLength(contents) > 1024 * 1024) throw new Error("The saved profiles file is too large.");
    const profiles = JSON.parse(contents);
    if (!Array.isArray(profiles) || profiles.length > 200) throw new Error("The saved profiles file is invalid.");
    const ids = new Set();
    return profiles.map((profile) => {
      const valid = validateProfile(profile, { requireConnection: true });
      if (!valid.id || !valid.kind || ids.has(valid.id)) throw new Error("The saved profiles file contains an invalid or duplicate profile.");
      ids.add(valid.id);
      return valid;
    });
  }
  write(profiles) {
    mkdirSync(path.dirname(this.filename), { recursive: true, mode: 0o700 });
    const temporary = `${this.filename}.${randomUUID()}.tmp`;
    try {
      writeFileSync(temporary, `${JSON.stringify(profiles, null, 2)}\n`, { encoding: "utf8", mode: 0o600, flag: "wx" });
      renameSync(temporary, this.filename);
    } finally { try { unlinkSync(temporary); } catch (error) { if (error.code !== "ENOENT") throw error; } }
  }
  save(input) {
    const profile = validateProfile(input, { requireConnection: true });
    if (!profile.kind) throw new Error("A profile kind is required.");
    profile.id ||= randomUUID();
    profile.name ||= profile.host || profile.device || (profile.kind === "agent" ? "Agent" : "Terminal");
    const profiles = this.list();
    const index = profiles.findIndex((item) => item.id === profile.id);
    if (index < 0) {
      if (profiles.length >= 200) throw new Error("A maximum of 200 profiles can be saved.");
      profiles.push(profile);
    } else profiles[index] = profile;
    this.write(profiles);
    return profiles;
  }
  delete(id) {
    if (typeof id !== "string" || !/^[a-zA-Z0-9_-]{1,80}$/.test(id)) throw new Error("Invalid profile id.");
    const profiles = this.list().filter((profile) => profile.id !== id);
    this.write(profiles);
    return profiles;
  }
}
