<a name="readme-top"></a>

<br />
<div align="center">
  <a href="https://smoo.ai">
    <img src="../../images/logo.png" alt="SmooAI Logo" />
  </a>
</div>

# @smooai/observability-react

![NPM Version](https://img.shields.io/npm/v/%40smooai%2Fobservability-react?style=for-the-badge)
![NPM Downloads](https://img.shields.io/npm/dw/%40smooai%2Fobservability-react?style=for-the-badge)
![GitHub License](https://img.shields.io/github/license/SmooAI/observability?style=for-the-badge)

React bindings for `@smooai/observability`. Drop in an `<ErrorBoundary>` and you're done — captured errors arrive in your Smoo dashboard with the component tree and React error info attached.

```sh
pnpm add @smooai/observability @smooai/observability-react
```

## API

### `<ErrorBoundary>`

```tsx
import { ErrorBoundary } from '@smooai/observability-react';

<ErrorBoundary fallback={<div>Something went wrong</div>}>
    <App />
</ErrorBoundary>;
```

Render-prop fallback for retry UI:

```tsx
<ErrorBoundary
    fallback={({ error, resetError }) => (
        <div>
            <p>{error.message}</p>
            <button onClick={resetError}>Try again</button>
        </div>
    )}
>
    <App />
</ErrorBoundary>
```

### `useErrorHandler()`

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
