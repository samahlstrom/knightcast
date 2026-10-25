# User-owned Knight integration

These are the user's existing assets and rig, not assets from the reference Windows application. Load them from an explicitly supplied `--asset-root` pointing at the user's stream-kit directory. Do not bundle the personal artwork in the public source repository.

## Existing files

- `overlays/head-blender/samson-knight-head.glb`: full rounded 3D helmet, painted materials, sculpted red plume, outlines, bolts, neck, eyes, jaw.
- `overlays/head-blender/rig.mjs`: user's existing center-pivot and collar fitting rules.
- `overlays/head-blender/live.mjs`: user's existing orthographic framing, materials, eye surface and jaw sleeve behavior.
- `overlays/original-rig/rig.json`: bones, slots, image paths, native sprite sizes, bind poses, IK targets and reach bounds.
- `overlays/original-rig/parts/*.png`: rigid cutout parts, not deformable meshes.
- `overlays/desk-rig/skeleton.js`: user's existing rigid cutout hierarchy and two-bone IK.

Reading/reimplementing the user's own code is allowed; it is separate from the clean-room behavioral reference. Preserve its visuals and geometry. Do not flatten the helmet or redraw the character.

## Coordinates and layering

Art is 1254 by 1300 pixels; 100 pixels = one world unit. Orthographic camera bounds left -6.04, right 6.50, top 7, bottom -6; camera at positive Z looking toward the origin. Body artwork plane center is (0.23, 0.5, 0). Art point (604,700) is world (0,0,0).

Move all original children of `HeadRotation` into a geometry assembly offset (0,-2.125,0), then place HeadRotation at (0,2.375,0). This rotates about the middle of the helmet while preserving all three dimensions. Keep its scale uniformly (1,1,1). `Neck*` meshes stay outside this rotation, with local Y scale 2 and position (0,-0.25,-0.66). This is the accepted collar fit.

Use right-handed quaternion rotations. The old rendering angle order was YXZ. Neutral calibration is inverse(neutral) * current, never subtraction of Euler components. Clamp meaningful angles gently to yaw +/-45 degrees, pitch +/-25 degrees and roll +/-24 degrees. Apply any near/far movement uniformly and coordinate it with torso lean.

For the body, render only slots below the 2D head's layer, preserving original order. Bound sprite slots transform around their bind point and native sourcePosition; non-bound slots use their bone transform, local slot translation, pivot, rotation and flips. Do not render the original 2D helmet/plume over the 3D helmet. Preserve black structural backing pieces hiding arm seams. Any pointer target must remain on the desk, and bone lengths must remain constant.

## Controls and expressions

Left mouse button presses right-hand index (`right_finger_0`); right button presses middle (`right_finger_1`). Typing activates the keyboard fingers. Head rotation must not depend on pointer or typing events. The stream-kit event hub on localhost:8790 can supply existing global input events without a browser; support can be added without modifying the live hub.

Opening inner lips slides the jaw down (up to 0.54 world units); microphone amplitude must not animate the jaw. Preserve white oval eyes, blinks, gaze, smiling arcs, wide-eye brows and the later surprise effect. Surprise is exactly three white pairs anchored at the eyes, scaling 1, 1.55, 2.15 and extending along local +Z (depth 0,1.35,2.85), not sideways. Do not silently replace unfinished expressions with a claim of full parity.

## Validation and user session

Use offscreen GPU renders and synthetic tracking for initial verification. Do not launch a visible window, change the user's focused window, alter OBS scenes or capture the camera during background implementation. A new native app can be launched when the user chooses to switch. Keep all live stream-kit services intact. Screenshots for verification belong outside the public repository.

## Convex sprite outlines

A sprite asset may include `convexOutline`, an ordered convex polygon of normalized
UV pairs in `[0,1]`. The native loader draws a triangle fan over that outline instead
of the default full image quad; it does not repaint or resample the source image.
This allows an extracted prop to exclude surrounding background scraps. Keep the
existing desk backing beneath any pixels that were previously part of a rectangular
prop cutout. `sourceRect` still selects a source region before UVs are applied.
