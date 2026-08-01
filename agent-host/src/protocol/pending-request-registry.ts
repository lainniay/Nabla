export interface PendingRegistration<T> {
  value: T;
  cleanup?: () => void;
}

/**
 * Owns the mechanical lifecycle shared by interactive host requests. Domain
 * adapters retain their distinct resolve/reject and fail-close semantics.
 */
export class PendingRequestRegistry<T> {
  private readonly entries = new Map<string, PendingRegistration<T>>();

  register(id: string, value: T, cleanup?: () => void): void {
    if (this.entries.has(id)) {
      throw new Error(`Pending request already exists: ${id}`);
    }
    this.entries.set(id, { value, cleanup });
  }

  take(id: string): T | undefined {
    const entry = this.entries.get(id);
    if (!entry) return undefined;
    this.entries.delete(id);
    entry.cleanup?.();
    return entry.value;
  }

  get(id: string): T | undefined {
    return this.entries.get(id)?.value;
  }

  drain(): T[] {
    const entries = [...this.entries.values()];
    this.entries.clear();
    for (const entry of entries) entry.cleanup?.();
    return entries.map((entry) => entry.value);
  }

  get size(): number {
    return this.entries.size;
  }
}
