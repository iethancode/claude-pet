import PetView from "./views/PetView";

/** Each window is a pet window (one per Claude Code session). The settings
 *  center was removed — pet switching is done by right-clicking the pet. */
export default function App() {
  return <PetView />;
}
