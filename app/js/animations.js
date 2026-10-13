// Web Animations the stylesheet cannot express. With "reduce motion" set,
// only the colour changes stay.

const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches

// A wrong key on manuscript paper: the square to type flashes vermilion
// and, motion allowed, shakes.
export function squareMistake(el) {
  el.animate([{ backgroundColor: 'var(--miss)' }, {}], { duration: 450, easing: 'ease-out' })
  if (reduced()) return
  el.animate(
    [{ translate: '3px 0' }, { translate: '-3px 0' }, { translate: '2px 0' }, { translate: '0 0' }],
    { duration: 200, easing: 'ease-in-out' },
  )
}
