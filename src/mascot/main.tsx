import React from "react";
import ReactDOM from "react-dom/client";
import "../shared/fonts";
import "../shared/base.css";
import "./mascot.css";
import Mascot from "./Mascot";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Mascot />
  </React.StrictMode>,
);
