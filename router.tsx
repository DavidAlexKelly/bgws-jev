import { lazy } from "react";
import { createBrowserRouter, Navigate } from "react-router-dom";
import AuthCallback from "@/AuthCallback";

// Battlegroup Wargame System. The screens are large (map, rules engine,
// real-time engine) so each is code-split; main.tsx wraps the router in a
// <Suspense> with the loading fallback.
//
// Paths are unchanged from the Mapplications repo (/bgws/…) because the BGWS
// screens link to them directly.
const BgwsHome = lazy(() => import("@/apps/bgws/Home"));
const BgwsPlay = lazy(() => import("@/apps/bgws/Play"));
const BgwsAssetExplorer = lazy(() => import("@/apps/bgws/AssetExplorer"));
const BgwsAssetHub = lazy(() => import("@/apps/bgws/AssetHub"));
const BgwsBehaviourManager = lazy(() => import("@/apps/bgws/BehaviourManager"));

export const router = createBrowserRouter(
  [
    {
      // BGWS is the whole app — land on its home screen.
      path: "/",
      element: <Navigate to="/bgws" replace />,
    },
    { path: "/bgws", element: <BgwsHome /> },
    { path: "/bgws/play", element: <BgwsPlay /> },
    { path: "/bgws/assets", element: <BgwsAssetHub /> },
    { path: "/bgws/assets/units", element: <BgwsAssetExplorer /> },
    { path: "/bgws/assets/behaviours", element: <BgwsBehaviourManager /> },
    {
      // This is the route defined in your application's redirect URL
      path: "/auth/callback",
      element: <AuthCallback />,
    },
    {
      // Includes the retired /bgws/board and /bgws/trial.
      path: "*",
      element: <Navigate to="/bgws" replace />,
    },
  ],
  { basename: import.meta.env.BASE_URL },
);
