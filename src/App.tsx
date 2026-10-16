import { Sidebar } from "./components/Sidebar";
import { Terminal } from "./components/Terminal";

export default function App() {
  return (
    <div className="app">
      <Sidebar />
      <main className="main">
        <Terminal />
      </main>
    </div>
  );
}
