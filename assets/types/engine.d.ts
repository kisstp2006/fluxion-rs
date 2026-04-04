// Auto-generated TypeScript declarations — do not edit

declare namespace Application {
  /** Quits the application. */
  function Quit(): void;
  /** Opens the specified URL in the default browser. */
  function OpenURL(url: string): void;
  /** Returns the product name set in project settings. */
  function getProductName(): string;
  /** Returns the company name. */
  function getCompanyName(): string;
  /** Returns the application version string. */
  function getVersion(): string;
  /** Returns the target frame rate (-1 = unlimited). */
  function getTargetFrameRate(): number;
  /** Sets the target frame rate. -1 = unlimited. */
  function setTargetFrameRate(rate: number): void;
  /** Returns the path to the application data folder. */
  function getDataPath(): string;
  /** Returns the runtime platform identifier string. */
  function getPlatform(): string;
  /** Sets the window title. */
  function setTitle(title: string): void;
  /** Returns the current window title. */
  function getTitle(): string;
}

declare namespace Cursor {
  /** Shows or hides the cursor. */
  function setVisible(visible: boolean): void;
  /** Sets cursor lock mode: 0=None, 1=Locked, 2=Confined. */
  function setLockMode(mode: number): void;
}

declare namespace Debug {
  /** Logs a message to the console. */
  function Log(message: string, context?: object): void;
  /** Logs a warning message. */
  function LogWarning(message: string): void;
  /** Logs an error message. */
  function LogError(message: string): void;
  /** Logs an error if condition is false. */
  function Assert(condition: boolean, message?: string): void;
  /** Draws a line in the scene view between start and end. */
  function DrawLine(start: Vector3, end: Vector3, color?: Vector4, duration?: number): void;
  /** Draws a ray from origin in direction. */
  function DrawRay(origin: Vector3, direction: Vector3, color?: Vector4, duration?: number): void;
  /** Pauses the editor (no-op in standalone builds). */
  function Break(): void;
  /** Clears errors from the developer console. */
  function ClearDeveloperConsole(): void;
}

declare namespace GUI {
  /** Draws a text label at the given Rect. */
  function Label(rect: object, text: string): void;
  /** Draws a button. Returns true on the frame it is clicked. */
  function Button(rect: object, text: string): boolean;
  /** Draws a box with optional label. */
  function Box(rect: object, text?: string): void;
  /** Draws a toggle. Returns the current value. */
  function Toggle(rect: object, value: boolean, text: string): boolean;
  /** Draws a horizontal slider. Returns the current value. */
  function HorizontalSlider(rect: object, value: number, min: number, max: number): number;
  /** Draws a single-line text field. Returns the current text. */
  function TextField(rect: object, text: string): string;
  /** Begins a group (clip region). */
  function BeginGroup(rect: object): void;
  /** Ends the current group. */
  function EndGroup(): void;
  /** Draws a texture inside a Rect. */
  function DrawTexture(rect: object, texturePath: string): void;
}

declare namespace GUILayout {
  /** Auto-layout label. */
  function Label(text: string): void;
  /** Auto-layout button. Returns true when clicked. */
  function Button(text: string): boolean;
}

declare namespace GameObject {
  /** Finds a GameObject by name. Returns null if not found. */
  function Find(name: string): object;
  /** Finds the first active GameObject tagged with tag. */
  function FindWithTag(tag: string): object;
  /** Returns an array of all active GameObjects tagged with tag. */
  function FindGameObjectsWithTag(tag: string): any[];
  /** Destroys the GameObject, component or asset. Optional delay in seconds. */
  function Destroy(objectName: string, delay?: number): void;
  /** Clones a prefab asset into the scene. Returns the new object's name. */
  function Instantiate(prefabPath: string, position?: Vector3, rotation?: Quaternion): string;
  /** Activates/deactivates the GameObject by name. */
  function SetActive(name: string, active: boolean): void;
}

declare namespace Input {
  /** Returns true while the user holds down the key identified by keyCode. */
  function GetKey(keyCode: string): boolean;
  /** Returns true during the frame the user starts pressing down the key. */
  function GetKeyDown(keyCode: string): boolean;
  /** Returns true during the frame the user releases the key. */
  function GetKeyUp(keyCode: string): boolean;
  /** Returns whether the given mouse button is held. 0=left, 1=right, 2=middle. */
  function GetMouseButton(button: number): boolean;
  /** Returns true during the frame the user pressed the mouse button. */
  function GetMouseButtonDown(button: number): boolean;
  /** Returns true during the frame the user released the mouse button. */
  function GetMouseButtonUp(button: number): boolean;
  /** Returns the value of the virtual axis identified by axisName (-1..1). Supports 'Horizontal', 'Vertical', 'Mouse X', 'Mouse Y'. */
  function GetAxis(axisName: string): number;
  /** Returns the value of the virtual axis with no smoothing applied. */
  function GetAxisRaw(axisName: string): number;
  /** Returns an array of strings describing connected joysticks. */
  function GetJoystickNames(): any[];
}

declare namespace Physics {
  /** Casts a ray and returns the first hit. Returns null if nothing was hit. */
  function Raycast(origin: Vector3, direction: Vector3, maxDistance?: number): object;
  /** Casts a ray and returns ALL hits sorted by distance. */
  function RaycastAll(origin: Vector3, direction: Vector3, maxDistance?: number): any[];
  /** Returns an array of colliders whose bounding volumes overlap the given sphere. */
  function OverlapSphere(center: Vector3, radius: number): any[];
  /** Returns colliders inside an axis-aligned box. */
  function OverlapBox(center: Vector3, halfExtents: Vector3): any[];
  /** Returns the gravity vector used by the physics simulation. */
  function getGravity(): Vector3;
  /** Sets the gravity vector for the physics simulation. */
  function setGravity(gravity: Vector3): void;
  /** Simulates physics by the given step size. Use carefully in FixedUpdate only. */
  function Simulate(step: number): void;
  /** Marks two colliders to ignore collisions with each other. */
  function IgnoreCollision(colliderA: object, colliderB: object, ignore?: boolean): void;
}

declare namespace SceneManager {
  /** Loads the scene at the given path. set additive=true to load without unloading current. */
  function LoadScene(scenePath: string, additive?: boolean): void;
  /** Returns an object describing the currently loaded scene. */
  function GetActiveScene(): object;
}

declare namespace Screen {
  /** Returns the current screen/window width in pixels. */
  function getWidth(): number;
  /** Returns the current screen/window height in pixels. */
  function getHeight(): number;
  /** Returns whether the application is running in full-screen mode. */
  function getFullScreen(): boolean;
  /** Enter or exit full-screen mode. */
  function setFullScreen(fullscreen: boolean): void;
  /** Sets the screen resolution. fullscreen is optional (default false). */
  function SetResolution(width: number, height: number, fullscreen?: boolean): void;
  /** Returns the approximate DPI of the screen. */
  function getDpi(): number;
}

declare namespace Time {
  /** Time in seconds it took to complete the last frame (scaled by timeScale). */
  function getDeltaTime(): number;
  /** The time at the beginning of this frame (seconds since level load). */
  function getTime(): number;
  /** The fixed time step used for physics and FixedUpdate. */
  function getFixedDeltaTime(): number;
  /** Sets the fixed time step (seconds). */
  function setFixedDeltaTime(value: number): void;
  /** The total number of frames rendered since the start of the application. */
  function getFrameCount(): number;
  /** The scale at which time passes. 1 = real time, 0 = paused, 2 = double speed. */
  function getTimeScale(): number;
  /** Sets the time scale. */
  function setTimeScale(scale: number): void;
  /** Delta time independent of timeScale. */
  function getUnscaledDeltaTime(): number;
  /** Elapsed time since start, independent of timeScale. */
  function getUnscaledTime(): number;
  /** Monotonic clock — wall-clock time since application startup. */
  function getRealtimeSinceStartup(): number;
}

