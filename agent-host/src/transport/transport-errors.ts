export class TransportError extends Error {}

export class JsonlParseError extends TransportError {}

export class JsonlRequestError extends TransportError {}

export class FrameTooLargeError extends TransportError {}
