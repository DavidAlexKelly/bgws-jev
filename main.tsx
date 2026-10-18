import { Suspense } from "react";
import ReactDOM from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import ErrorBoundary from "@/components/ErrorBoundary";
import Loading from "@/components/Loading";
import { router } from "@/router";
import { setupBasemap } from "@/shared/map/dechoBasemapSetup";
import "./index.css";

// Hands this app's OAuth client and Foundry origin to @acc/decho-basemap.
// Must run before any map mounts; doing it here covers every route.
setupBasemap();

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("Root element not found");
}

ReactDOM.createRoot(rootElement).render(
  <ErrorBoundary>
    <Suspense fallback={<Loading />}>
      <RouterProvider router={router} />
    </Suspense>
  </ErrorBoundary>,
);
