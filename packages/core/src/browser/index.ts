/**
 * Browser entry — registers global capture handlers and a beacon-aware
 * batched HTTP transport.
 *
 * This file is the integration surface. The capture handlers themselves and
 * the breadcrumb wrappers live in sibling files and are wired here.
 */
export { Client, Scope, withScope, getCurrentScope } from '../index';
export * from '../types';

// TODO (SMOODEV-1067 follow-ups):
//   - registerBrowserGlobalHandlers()  → window.onerror, unhandledrejection, console.error tap
//   - installFetchBreadcrumbs()         → wrap window.fetch + XHR
//   - installNavigationBreadcrumbs()    → history.pushState, popstate, hashchange
//   - installClickBreadcrumbs()         → document.addEventListener('click', ...)
//   - browserStackParser()              → Chrome/Firefox/Safari format normalization
//   - browserTransport()                → batched fetch + sendBeacon on pagehide
//   - registerOfflineQueue()            → IndexedDB-backed queue with retry on focus
