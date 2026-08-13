import { HashRouter } from "react-router-dom";
import { AppShell } from "./components/AppShell";

function App() {
    return (
        <HashRouter>
            <AppShell />
        </HashRouter>
    );
}

export default App;
