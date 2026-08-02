#!/usr/bin/env python3
"""
Simple Snake game implemented with pygame.

How to run:
    pip install pygame
    python snake_game.py

Controls:
    Arrow keys to move the snake.
    Press ESC or close the window to quit.

The game is deliberately minimalistic but fully functional:
    - Growing snake when eating food.
    - Game over on wall or self-collision.
    - Score display.
"""

import pygame
import sys
import random
from collections import deque

# --------------------------------------------------------------
# Configuration constants
# --------------------------------------------------------------
WINDOW_WIDTH = 640
WINDOW_HEIGHT = 480
GRID_SIZE = 20
GRID_WIDTH = WINDOW_WIDTH // GRID_SIZE
GRID_HEIGHT = WINDOW_HEIGHT // GRID_SIZE
FPS = 10  # speed of the snake

# Derived pixel dimensions
BLOCK_SIZE = GRID_SIZE

# Colors (R, G, B)
BLACK = (0, 0, 0)
WHITE = (255, 255, 255)
GREEN = (0, 255, 0)
RED = (255, 0, 0)
DARK_GREEN = (0, 200, 0)
GRAY = (50, 50, 50)


# --------------------------------------------------------------
# Helper functions
# --------------------------------------------------------------
def draw_rect(surface, color, pos):
    """Draw a single grid block."""
    x, y = pos
    rectangle = pygame.Rect(x * BLOCK_SIZE, y * BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE)
    pygame.draw.rect(surface, color, rectangle)


def random_food_position(snake_body):
    """Return a random position not occupied by the snake."""
    while True:
        pos = (random.randint(0, GRID_WIDTH - 1), random.randint(0, GRID_HEIGHT - 1))
        if pos not in snake_body:
            return pos


# --------------------------------------------------------------
# Snake class
# --------------------------------------------------------------
class Snake:
    """Represents the snake and its behavior."""

    def __init__(self):
        # Start in the middle of the screen
        start_x = GRID_WIDTH // 2
        start_y = GRID_HEIGHT // 2
        self.body = deque(
            [
                (start_x, start_y),
                (start_x - 1, start_y),
                (start_x - 2, start_y),
            ]
        )
        self.direction = (1, 0)  # Initially moving right
        self.grow_pending = 0

    def set_direction(self, new_dir):
        """Change direction unless it would be a 180° turn."""
        opposite = (-self.direction[0], -self.direction[1])
        if new_dir != opposite:
            self.direction = new_dir

    def move(self):
        """Move the snake one step in the current direction."""
        head_x, head_y = self.body[0]
        dx, dy = self.direction
        new_head = (head_x + dx, head_y + dy)

        # Check wall collision
        if (
            new_head[0] < 0
            or new_head[0] >= GRID_WIDTH
            or new_head[1] < 0
            or new_head[1] >= GRID_HEIGHT
        ):
            return False  # game over

        # Check self-collision
        if new_head in self.body:
            return False  # game over

        self.body.appendleft(new_head)

        if self.grow_pending > 0:
            self.grow_pending -= 1  # keep the tail (grow)
        else:
            self.body.pop()  # normal move, remove tail

        return True

    def grow(self, amount=1):
        """Increase length by pending growth steps."""
        self.grow_pending += amount

    def draw(self, surface):
        """Render the snake on the given surface."""
        for segment in self.body:
            draw_rect(
                surface, GREEN if segment == self.body[0] else DARK_GREEN, segment
            )


# --------------------------------------------------------------
# Main game function
# --------------------------------------------------------------
def main():
    pygame.init()
    screen = pygame.display.set_mode((WINDOW_WIDTH, WINDOW_HEIGHT))
    pygame.display.set_caption("Snake")
    clock = pygame.time.Clock()

    # Initialise game objects
    snake = Snake()
    food_pos = random_food_position(snake.body)
    score = 0
    font = pygame.font.SysFont(None, 36)

    # Custom event for feeding the snake
    FEED_EVENT = pygame.USEREVENT + 1
    pygame.time.set_timer(FEED_EVENT, 1000)  # not used; just example

    running = True
    while running:
        # ------------------------------------------------------
        # Event handling
        # ------------------------------------------------------
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                running = False

            elif event.type == pygame.KEYDOWN:
                if event.key == pygame.K_ESCAPE:
                    running = False
                elif event.key == pygame.K_UP:
                    snake.set_direction((0, -1))
                elif event.key == pygame.K_DOWN:
                    snake.set_direction((0, 1))
                elif event.key == pygame.K_LEFT:
                    snake.set_direction((-1, 0))
                elif event.key == pygame.K_RIGHT:
                    snake.set_direction((1, 0))

        # ------------------------------------------------------
        # Game logic
        # ------------------------------------------------------
        if not snake.move():
            # Game over handling
            game_over_text = font.render("Game Over! Press R to restart", True, RED)
            screen.blit(
                game_over_text,
                (
                    WINDOW_WIDTH // 2 - game_over_text.get_width() // 2,
                    WINDOW_HEIGHT // 2 - game_over_text.get_height() // 2,
                ),
            )
            pygame.display.flip()

            # Wait for R to restart or Q to quit
            waiting = True
            while waiting:
                for ev in pygame.event.get():
                    if ev.type == pygame.QUIT:
                        waiting = False
                        running = False
                    elif ev.type == pygame.KEYDOWN:
                        if ev.key == pygame.K_r:
                            # Reset game state
                            snake = Snake()
                            food_pos = random_food_position(snake.body)
                            score = 0
                            waiting = False
                        elif ev.key == pygame.K_q:
                            waiting = False
                            running = False

            continue

        # Check if snake ate food
        if snake.body[0] == food_pos:
            snake.grow()
            score += 1
            food_pos = random_food_position(snake.body)
            # Reset timer event if you want periodic feeding
            # pygame.time.set_timer(FEED_EVENT, 1000)

        # ------------------------------------------------------
        # Rendering
        # ------------------------------------------------------
        screen.fill(BLACK)

        # Draw food
        draw_rect(screen, RED, food_pos)

        # Draw snake
        snake.draw(screen)

        # Draw score
        score_text = font.render(f"Score: {score}", True, WHITE)
        screen.blit(score_text, (10, 10))

        pygame.display.flip()
        clock.tick(FPS)

    pygame.quit()
    sys.exit()


if __name__ == "__main__":
    main()
