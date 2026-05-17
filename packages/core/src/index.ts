/**
 * Universal entry point. Re-exports types and runtime-agnostic helpers.
 * Browser and Node entry points expose the runtime-specific `Client.init`.
 */
export * from './types';
export { Scope, withScope, getCurrentScope } from './scope';
export { Client } from './client';
