import React from "react";
import ReactDOM from "react-dom/client";
import "../shared/fonts";
import "../shared/base.css";
import "./popup.css";
import Popup from "./Popup";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Popup />
  </React.StrictMode>,
);
