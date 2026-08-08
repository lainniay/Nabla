export class Deferred<T> {
  private value: T | undefined;

  bind(value: T): void {
    this.value = value;
  }

  get(): T {
    if (this.value === undefined) {
      throw new Error("Deferred value is not bound");
    }
    return this.value;
  }
}
