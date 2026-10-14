import React from "react";
import ReactDOM from "react-dom/client";
import "../shared/fonts";
import "./settings.css";
import Settings from "./Settings";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Settings />
  </React.StrictMode>,
);
