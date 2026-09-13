//! Keyboard actions for the viewer.

use gpui::actions;

actions!(
    sh_images,
    [
        NextImage,
        PrevImage,
        ToggleOverlays,
        ToggleFullscreen,
        OpenFile,
        OpenFolder,
        BackToGrid,
        OpenSelected,
        ToggleCrop
    ]
);
