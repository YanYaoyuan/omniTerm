// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Lists the hosts in ~/.ssh/config so they can be offered for import. Imported
// profiles keep only the alias and let ssh resolve it, so this file never has to
// reproduce ssh's behavior: what it reads here only labels the import list.
import { readdirSync, readFileSync, statSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const MAX_DEPTH = 16;
const MAX_BYTES = 1024 * 1024;
const MAX_HOSTS = 1000;
const WILDCARD = /[*?]/;
const GLOB = /[*?[]/;

// Splits a directive the way ssh does: `Key value`, `Key=value`, quoted
// arguments, and `#` comments outside quotes.
export function tokenize(line) {
  const tokens = [];
  let current = null;
  let quote = false;
  for (const character of line.trim()) {
    if (character === '"') { quote = !quote; current ??= ""; }
    else if (!quote && character === "#") break;
    // Only the keyword may be separated by `=`; values can contain it.
    else if (!quote && (/\s/.test(character) || (character === "=" && !tokens.length))) { if (current !== null) tokens.push(current); current = null; }
    else current = (current ?? "") + character;
  }
  if (current !== null) tokens.push(current);
  return tokens;
}

function globRegex(pattern) {
  return new RegExp(`^${pattern.replace(/[.+^${}()|\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")}$`);
}

// Expands one path segment at a time, sorted like glob(3).
function expandGlob(pattern, { list }) {
  const { root } = path.parse(pattern);
  let matches = [root];
  for (const segment of pattern.slice(root.length).split(path.sep).filter(Boolean)) {
    if (!GLOB.test(segment)) { matches = matches.map((base) => path.join(base, segment)); continue; }
    const regex = globRegex(segment);
    matches = matches.flatMap((base) => {
      try { return list(base).filter((name) => regex.test(name) && (segment.startsWith(".") || !name.startsWith("."))).sort().map((name) => path.join(base, name)); }
      catch { return []; }
    });
  }
  return matches;
}

function expandHome(value, home) {
  if (value === "~") return home;
  return value.startsWith("~/") || value.startsWith("~\\") ? path.join(home, value.slice(2)) : value;
}

// Yields sections in file order, with Include files spliced in where they appear.
function readSections(filename, context, depth) {
  if (depth > MAX_DEPTH || context.visited.has(filename)) return;
  context.visited.add(filename);
  const contents = context.read(filename);
  if (contents === null) return;
  for (const line of contents.replace(/^﻿/, "").split(/\r?\n/)) {
    const [keyword, ...values] = tokenize(line);
    if (!keyword) continue;
    const key = keyword.toLowerCase();
    if (key === "host") context.sections.push(context.section = { patterns: values, options: {} });
    else if (key === "match") context.sections.push(context.section = { patterns: [], options: {} });
    else if (key === "include") {
      const current = context.section;
      for (const value of values) {
        const expanded = expandHome(value, context.home);
        const absolute = path.isAbsolute(expanded) ? expanded : path.join(context.home, ".ssh", expanded);
        for (const match of GLOB.test(absolute) ? expandGlob(absolute, context) : [absolute]) readSections(match, context, depth + 1);
      }
      // An included file's sections end with it; the including section continues.
      context.sections.push(context.section = { patterns: current.patterns, options: {} });
    } else if (values.length && !(key in context.section.options)) context.section.options[key] = values[0];
  }
}

function matchesHost(patterns, alias) {
  let matched = false;
  for (const pattern of patterns) {
    const negated = pattern.startsWith("!");
    if (!globRegex(negated ? pattern.slice(1) : pattern).test(alias)) continue;
    if (negated) return false;
    matched = true;
  }
  return matched;
}

function resolveHost(alias, sections) {
  const options = {};
  for (const section of sections) if (matchesHost(section.patterns, alias)) for (const [key, value] of Object.entries(section.options)) options[key] ??= value;
  const port = Number(options.port);
  return {
    alias,
    hostname: (options.hostname ?? alias).replace(/%(%|h)/g, (_match, token) => token === "h" ? alias : "%"),
    ...(options.user ? { user: options.user } : {}),
    ...(port > 0 && port <= 65535 ? { port } : {}),
    ...(options.proxyjump && options.proxyjump.toLowerCase() !== "none" ? { proxyJump: options.proxyjump } : {}),
  };
}

export function readSshHosts({ home = os.homedir(), filename = path.join(home, ".ssh", "config"), read, list = (directory) => readdirSync(directory) } = {}) {
  let failure = "";
  const readFile = read ?? ((target) => {
    try {
      if (!statSync(target).isFile()) return null;
      if (statSync(target).size > MAX_BYTES) throw new Error(`${target} is too large to read.`);
      return readFileSync(target, "utf8");
    } catch (error) {
      if (error.code !== "ENOENT" && target === filename) failure = error.message;
      return null;
    }
  });
  const section = { patterns: ["*"], options: {} };
  const context = { home, read: readFile, list, visited: new Set(), sections: [section], section };
  readSections(filename, context, 0);
  const aliases = new Set();
  for (const { patterns } of context.sections) {
    for (const pattern of patterns) if (!pattern.startsWith("!") && !WILDCARD.test(pattern)) aliases.add(pattern);
  }
  return { filename, error: failure, hosts: [...aliases].slice(0, MAX_HOSTS).map((alias) => resolveHost(alias, context.sections)) };
}

export function hostSummary(host) {
  const target = `${host.user ? `${host.user}@` : ""}${host.hostname}${host.port ? `:${host.port}` : ""}`;
  return host.proxyJump ? `${target} · 经 ${host.proxyJump}` : target;
}
