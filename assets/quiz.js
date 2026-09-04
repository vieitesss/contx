// contx course — quiz widget. One correct choice per .quiz block.
// Markup contract:
//   <div class="quiz" data-answer="x" data-correct="..." data-incorrect="...">
//     <button data-choice="x">...</button>
//     <p class="feedback"></p>
//   </div>
document.querySelectorAll(".quiz").forEach((quiz) => {
  const answer = quiz.dataset.answer;
  const feedback = quiz.querySelector(".feedback");
  quiz.querySelectorAll("button[data-choice]").forEach((btn) => {
    btn.addEventListener("click", () => {
      quiz.querySelectorAll("button").forEach((b) =>
        b.setAttribute("aria-pressed", b === btn ? "true" : "false")
      );
      const correct = btn.dataset.choice === answer;
      feedback.textContent = correct ? quiz.dataset.correct : quiz.dataset.incorrect;
      feedback.className = "feedback " + (correct ? "ok" : "no");
    });
  });
});
