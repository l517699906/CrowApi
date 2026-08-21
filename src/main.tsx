import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { RootErrorBoundary } from "./components/ErrorBoundary";
import "./App.css";

const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            retry: 1,
            staleTime: 2_000,
            refetchOnWindowFocus: true,
        },
    },
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RootErrorBoundary>
        <App />
      </RootErrorBoundary>
    </QueryClientProvider>
  </React.StrictMode>,
);
