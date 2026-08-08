import type { PermissionRule } from "./model.ts";

export class PolicyStore {
  private builtin: PermissionRule[] = [];
  private managed: PermissionRule[] = [];
  private user: PermissionRule[] = [];
  private project: PermissionRule[] = [];
  private revisionValue = 0;

  get revision(): number {
    return this.revisionValue;
  }

  private bump(): void {
    this.revisionValue += 1;
  }

  setBuiltin(rules: readonly PermissionRule[]): void {
    this.builtin = rules.map((rule) => ({ ...rule, source: "builtin" }));
    this.bump();
  }

  setManaged(rules: readonly PermissionRule[]): void {
    this.managed = rules.map((rule) => ({ ...rule, source: "managed" }));
    this.bump();
  }

  setUser(rules: readonly PermissionRule[]): void {
    this.user = rules.map((rule) => ({ ...rule, source: "user" }));
    this.bump();
  }

  setProject(rules: readonly PermissionRule[]): void {
    this.project = rules
      .filter((rule) => rule.effect !== "allow")
      .map((rule) => ({ ...rule, source: "workspace" }));
    this.bump();
  }

  all(): PermissionRule[] {
    return [...this.builtin, ...this.managed, ...this.user, ...this.project];
  }
}
