# Persistent Transcript Surface

The Primary terminal surface is a physical projection of the session transcript. Nabla owns the
entire visible Primary screen from startup, while only the fixed `history_window` participates in
native terminal scrollback. The composer, status line, and panel overlay never enter that scroll
region.

## Terms

- **Canonical transcript** is the semantic session data reconstructed from Pi events. It is the
  source for resume, the transcript viewer, and destructive reflow.
- **Physical scrollback cursor** is the component cursor plus row offset that records which
  canonical rows have successfully entered native terminal scrollback. It advances only after the
  terminal write and flush succeed.
- **Resident transcript tail** is the bottom-aligned set of stable, sealed, and streaming rows
  currently projected into the fixed Primary `history_window`. Resident describes a screen
  position, not a component phase.
- **Bootstrap projection padding** is the application-owned blank space above a short resident
  tail. It is physical padding only: it is not canonical session data and is excluded from resume,
  the transcript viewer, and canonical reflow data.
- **Mutable streaming tail** is any row whose canonical component can still change. Streaming rows
  are always resident or clipped by the resident window; they never enter native scrollback.

`Stable` means that a row is eligible for overflow once newer content pushes it out of the
resident window. It does not mean that the row has already been written to native scrollback.
`Committed` means the row has been physically written and acknowledged. Changing a component from
`Streaming` to `Stable` or `Sealed` changes eligibility only and does not move its currently visible
rows.

## Projection and commit

`project_primary` is the single Primary projection entry point. It computes row-level
`overflow_blocks`, `resident_rows`, bootstrap padding, resident capacity, and the current physical
cursor. Components may overflow in multiple row slices without duplication.

A normal overflow commit sets the terminal scroll region to the fixed `history_window`, releases
bootstrap blanks from its top, scrolls overflow rows in canonical order, updates the Primary shadow
screen, redraws the complete resident frame and footer, then draws the panel overlay. Normal
Primary commits do not use Reverse Index or dynamically move the viewport.

If any write or flush fails, the physical cursor is not acknowledged and the projection becomes
invalid. Resize, resume/session replacement, and terminal failure use destructive canonical
recovery: native history is rebuilt from canonical overflow and the final screen is drawn from the
same resident projection used by normal rendering.
