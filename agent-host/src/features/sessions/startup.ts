import { SessionManager } from "@earendil-works/pi-coding-agent";

export function createStartupSessionManager(
  cwd: string,
  sessionDir?: string,
): SessionManager {
  return SessionManager.create(cwd, sessionDir);
}
