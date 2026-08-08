import { PendingRequestRegistry } from "../../protocol/pending-request-registry.ts";

export interface RequestQueuePending<TRequest, TReply> {
  request: TRequest;
  resolve(value: TReply): void;
  reject(error: Error): void;
}

export interface RequestQueueHooks<TReply> {
  onAbort(
    pending: {
      resolve(value: TReply): void;
      reject(error: Error): void;
    },
    announced: boolean,
  ): void;
  onNotifyError(
    pending: {
      resolve(value: TReply): void;
      reject(error: Error): void;
    },
    error: unknown,
  ): void;
}

/**
 * One pending-request queue used by the approval, question, and auth-prompt
 * flows. Callers keep their distinct abort/notify-failure semantics through
 * the hooks.
 */
export class RequestQueue<TRequest, TReply> {
  private readonly pending = new PendingRequestRegistry<
    RequestQueuePending<TRequest, TReply>
  >();

  request(
    requestId: string,
    request: TRequest,
    signals: readonly (AbortSignal | undefined)[],
    notify: () => void,
    hooks: RequestQueueHooks<TReply>,
  ): Promise<TReply> {
    return new Promise<TReply>((resolve, reject) => {
      let announced = false;
      const entry: RequestQueuePending<TRequest, TReply> = {
        request,
        resolve,
        reject,
      };
      const removeAbortListeners = () => {
        for (const signal of signals) {
          signal?.removeEventListener("abort", onAbort);
        }
      };
      const onAbort = () => {
        const aborted = this.pending.take(requestId);
        if (!aborted) return;
        hooks.onAbort(aborted, announced);
      };

      this.pending.register(requestId, entry, removeAbortListeners);
      for (const signal of signals) {
        signal?.addEventListener("abort", onAbort, { once: true });
      }

      // A signal can be aborted between the caller's check and listener
      // registration. Re-check so onAbort can consume the queue entry.
      if (signals.some((signal) => signal?.aborted)) {
        onAbort();
        return;
      }

      try {
        announced = true;
        notify();
      } catch (error) {
        const failed = this.pending.take(requestId);
        if (failed) {
          hooks.onNotifyError(failed, error);
        }
      }
    });
  }

  get(requestId: string): RequestQueuePending<TRequest, TReply> | undefined {
    return this.pending.get(requestId);
  }

  reply(requestId: string, value: TReply): boolean {
    const entry = this.pending.take(requestId);
    if (!entry) return false;
    entry.resolve(value);
    return true;
  }

  settleAll(settle: (entry: RequestQueuePending<TRequest, TReply>) => void): void {
    for (const entry of this.pending.drain()) settle(entry);
  }
}
