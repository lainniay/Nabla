interface PendingPrompt {
  resolve(value: string): void;
  reject(error: Error): void;
}

export class AuthPromptQueue {
  private readonly pending = new PendingRequestRegistry<PendingPrompt>();

  request(
    promptId: string,
    signals: readonly (AbortSignal | undefined)[],
    notify: () => void,
    onCancelled: () => void,
  ): Promise<string> {
    if (signals.some((signal) => signal?.aborted)) {
      return Promise.reject(new Error("Login cancelled"));
    }

    return new Promise<string>((resolve, reject) => {
      let announced = false;
      const removeAbortListeners = () => {
        for (const signal of signals) {
          signal?.removeEventListener("abort", onAbort);
        }
      };
      const pending: PendingPrompt = {
        resolve,
        reject,
      };
      const onAbort = () => {
        const aborted = this.pending.take(promptId);
        if (!aborted) return;
        if (announced) onCancelled();
        aborted.reject(new Error("Login cancelled"));
      };

      this.pending.register(promptId, pending, removeAbortListeners);
      for (const signal of signals) {
        signal?.addEventListener("abort", onAbort, { once: true });
      }

      // A signal can be aborted between the first check and listener
      // registration. At this point onAbort can safely consume the queue entry.
      if (signals.some((signal) => signal?.aborted)) {
        onAbort();
        return;
      }

      try {
        announced = true;
        notify();
      } catch (error) {
        this.pending
          .take(promptId)
          ?.reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  reply(promptId: string, value: string): boolean {
    const pending = this.pending.take(promptId);
    if (!pending) return false;
    pending.resolve(value);
    return true;
  }

  cancelAll(reason = "Login flow ended"): void {
    for (const pending of this.pending.drain()) {
      pending.reject(new Error(reason));
    }
  }
}
import { PendingRequestRegistry } from "./protocol/pending-request-registry.ts";
