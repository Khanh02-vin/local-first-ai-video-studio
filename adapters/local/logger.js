/**
 * Structured logging — minimal wrapper for consistent JSON logs.
 * Usage: import { log } from "./logger.js"; log.info("message", { key: "value" });
 */

const Level = { debug: 0, info: 1, warn: 2, error: 3 };

function format(entry) {
  return JSON.stringify(entry);
}

function createLogger(level) {
  const levels = { debug: 0, info: 1, warn: 2, error: 3 };
  const currentLevel = levels[level] ?? 1;
  
  return {
    debug: (msg, ctx) => { if (currentLevel <= 0) console.log(format({ level: "debug", message: msg, timestamp: new Date().toISOString(), context: ctx })); },
    info: (msg, ctx) => { if (currentLevel <= 1) console.log(format({ level: "info", message: msg, timestamp: new Date().toISOString(), context: ctx })); },
    warn: (msg, ctx) => { if (currentLevel <= 2) console.warn(format({ level: "warn", message: msg, timestamp: new Date().toISOString(), context: ctx })); },
    error: (msg, ctx) => { if (currentLevel <= 3) console.error(format({ level: "error", message: msg, timestamp: new Date().toISOString(), context: ctx })); },
  };
}

export const log = createLogger(process.env.LOG_LEVEL ?? "info");
export { Level };