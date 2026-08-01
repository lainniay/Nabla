export class CommandLanes {
  private readonly tails = new Map<string, Promise<void>>();

  run<T>(lane: string | undefined, action: () => Promise<T>): Promise<T> {
    if (!lane) return action();
    const previous = this.tails.get(lane) ?? Promise.resolve();
    const result = previous.then(action, action);
    const tail = result.then(
      () => undefined,
      () => undefined,
    );
    this.tails.set(lane, tail);
    void tail.then(() => {
      if (this.tails.get(lane) === tail) this.tails.delete(lane);
    });
    return result;
  }
}
