import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ContextWindow from "./ContextWindow";
import { I18nProvider } from "./i18n";
import "./index.css";

const currentView = new URLSearchParams(window.location.search).get("view");
const RootComponent = currentView === "workspace" ? ContextWindow : App;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <I18nProvider>
      <RootComponent />
    </I18nProvider>
  </React.StrictMode>,
);
