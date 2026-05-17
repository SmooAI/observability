# @smooai/observability-react

React bindings for `@smooai/observability`.

```
npm i @smooai/observability @smooai/observability-react
```

## API

### `<ErrorBoundary>`

```tsx
import { ErrorBoundary } from '@smooai/observability-react';

<ErrorBoundary fallback={<div>Something went wrong</div>}>{children}</ErrorBoundary>;
```

With a render-fallback:

```tsx
<ErrorBoundary
    fallback={({ error, resetError }) => (
        <div>
            <p>{error.message}</p>
            <button onClick={resetError}>Try again</button>
        </div>
    )}
>
    {children}
</ErrorBoundary>;
```

### `useErrorHandler()`

For capturing errors raised inside async event handlers where a render-time boundary cannot help:

```tsx
const handleError = useErrorHandler();

async function onSubmit() {
    try {
        await save();
    } catch (e) {
        handleError(e);
    }
}
```

## License

MIT
