export interface OperationContext {
  requestId?: string;
  connectionId: string;
  connectionGeneration: number;
  sessionId?: string;
  sessionGeneration: number;
  signal: AbortSignal;
}
