import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import StudioPreview from "./StudioPreview";

createRoot(document.getElementById("root")!).render(<StrictMode><StudioPreview /></StrictMode>);
