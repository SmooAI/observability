/**
 * Node entry — registers uncaughtException / unhandledRejection handlers and
 * exposes a Hono middleware. Transport is a batched fetch with AsyncLocalStorage
 * scope propagation per request.
 */
export { Client, Scope, withScope, getCurrentScope } from '../index';
export * from '../types';

// TODO (SMOODEV-1067 follow-ups):
//   - registerNodeGlobalHandlers()      → process.on('uncaughtException' | 'unhandledRejection')
//   - nodeStackParser()                  → v8 format
//   - nodeTransport()                    → batched fetch via undici, in-memory queue
//   - observabilityMiddleware()          → Hono middleware that pushes a per-request Scope
//                                          via AsyncLocalStorage, captures errors propagating
//                                          to the global onError handler
