/* @refresh reload */
import { render } from "solid-js/web";
import "./styles.css";
import { App } from "./App";

const root = document.getElementById("root");
if (!root) throw new Error("#root 미존재 (index.html 확인)");
render(() => <App />, root);
