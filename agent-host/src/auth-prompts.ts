import { RequestQueue } from "./features/interactions/request-queue.ts";
import { asError } from "./protocol/validation.ts";

export class AuthPromptQueue {
  private readonly queue = new RequestQueue<undefined, string>();

  request(
    promptId: string,
    signals: readonly (AbortSignal | undefined)[],
    notify: () => void,
    onCancelled: () => void,
  ): Promise<string> {
    return this.queue.request(
      promptId,
      undefined,
      signals,
      () => notify(),
      {
        onAbort: (pending, announced) => {
          if (announced) onCancelled();
          pending.reject(new Error("Login cancelled"));
        },
        onNotifyError: (pending, error) => pending.reject(asError(error)),
      },
    );
  }

  reply(promptId: string, value: string): boolean {
    return this.queue.reply(promptId, value);
  }

  cancelAll(reason = "Login flow ended"): void {
    this.queue.settleAll((pending) => pending.reject(new Error(reason)));
  }
}
