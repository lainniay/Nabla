export class HostDiagnostics {
  private readonly warnings: string[] = [];

  warn(message: string, _context?: Record<string, unknown>): void {
    if (!this.warnings.includes(message)) this.warnings.push(message);
  }

  snapshot(): readonly string[] {
    return [...this.warnings];
  }
}
