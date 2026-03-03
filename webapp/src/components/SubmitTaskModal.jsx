import { useEffect } from 'react'

export default function SubmitTaskModal({ description, setDescription, operatorToken, setOperatorToken, auth, onSubmit, onClose, submitError }) {
  useEffect(() => {
    const handler = (e) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={e => e.stopPropagation()}>
        <div className="modal-title">Submit Task</div>

        <textarea
          className="input"
          rows={4}
          placeholder="Describe the task…"
          value={description}
          onChange={e => setDescription(e.target.value)}
          autoFocus
        />

        {auth?.token_required && (
          <input
            className="input"
            placeholder="Operator token"
            type="password"
            value={operatorToken}
            onChange={e => setOperatorToken(e.target.value)}
          />
        )}

        {submitError && <div className="error-msg">{submitError}</div>}

        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn btn-primary" onClick={onSubmit} disabled={!description.trim()}>
            Submit
          </button>
        </div>
      </div>
    </div>
  )
}
