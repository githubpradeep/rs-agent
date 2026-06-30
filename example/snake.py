import pygame
import random
import sys

# Initialize Pygame
pygame.init()

# Constants
WIDTH, HEIGHT = 800, 600
CELL_SIZE = 20
GRID_WIDTH = WIDTH // CELL_SIZE
GRID_HEIGHT = HEIGHT // CELL_SIZE
FPS = 10

# Colors
BLACK = (0, 0, 0)
WHITE = (255, 255, 255)
GREEN = (0, 255, 0)
RED = (255, 0, 0)
BLUE = (0, 0, 255)

# Directions
UP = (0, -1)
DOWN = (0, 1)
LEFT = (-1, 0)
RIGHT = (1, 0)

class Snake:
    def __init__(self):
        self.length = 1
        # Start in the middle of the grid
        start_x = GRID_WIDTH // 2
        start_y = GRID_HEIGHT // 2
        self.positions = [(start_x, start_y)]
        self.direction = RIGHT
        self.grow_flag = False

    def change_direction(self, new_dir):
        # Prevent reversing into itself
        if (new_dir[0] * -1, new_dir[1] * -1) != self.direction:
            self.direction = new_dir

    def move(self):
        head = self.positions[0]
        new_head = (head[0] + self.direction[0], head[1] + self.direction[1])
        self.positions.insert(0, new_head)
        if not self.grow_flag:
            self.positions.pop()
        else:
            self.grow_flag = False
            self.length += 1

    def grow(self):
        self.grow_flag = True

    def check_collision(self):
        head = self.positions[0]
        # Wall collision
        if head[0] < 0 or head[0] >= GRID_WIDTH or head[1] < 0 or head[1] >= GRID_HEIGHT:
            return True
        # Self collision
        if head in self.positions[1:]:
            return True
        return False

    def get_head(self):
        return self.positions[0]

    def draw(self, surface):
        for i, pos in enumerate(self.positions):
            rect = pygame.Rect(pos[0] * CELL_SIZE, pos[1] * CELL_SIZE, CELL_SIZE, CELL_SIZE)
            if i == 0:
                # Head is slightly brighter
                pygame.draw.rect(surface, (0, 200, 0), rect)
                pygame.draw.rect(surface, GREEN, rect, 2)
            else:
                pygame.draw.rect(surface, GREEN, rect)
                pygame.draw.rect(surface, (0, 100, 0), rect, 1)

class Food:
    def __init__(self):
        self.position = (0, 0)
        self.randomize()

    def randomize(self, snake_positions=None):
        while True:
            x = random.randint(0, GRID_WIDTH - 1)
            y = random.randint(0, GRID_HEIGHT - 1)
            self.position = (x, y)
            # Avoid spawning on the snake
            if snake_positions is None or self.position not in snake_positions:
                break

    def draw(self, surface):
        rect = pygame.Rect(self.position[0] * CELL_SIZE, self.position[1] * CELL_SIZE, CELL_SIZE, CELL_SIZE)
        pygame.draw.rect(surface, RED, rect)
        pygame.draw.rect(surface, (200, 0, 0), rect, 2)

def show_game_over(screen, score):
    font_large = pygame.font.Font(None, 74)
    font_small = pygame.font.Font(None, 36)

    overlay = pygame.Surface((WIDTH, HEIGHT))
    overlay.set_alpha(200)
    overlay.fill(BLACK)
    screen.blit(overlay, (0, 0))

    text = font_large.render("GAME OVER", True, RED)
    text_rect = text.get_rect(center=(WIDTH // 2, HEIGHT // 2 - 50))
    screen.blit(text, text_rect)

    score_text = font_small.render(f"Score: {score}", True, WHITE)
    score_rect = score_text.get_rect(center=(WIDTH // 2, HEIGHT // 2 + 20))
    screen.blit(score_text, score_rect)

    restart_text = font_small.render("Press R to restart or Q to quit", True, WHITE)
    restart_rect = restart_text.get_rect(center=(WIDTH // 2, HEIGHT // 2 + 70))
    screen.blit(restart_text, restart_rect)

    pygame.display.flip()

    while True:
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                pygame.quit()
                sys.exit()
            if event.type == pygame.KEYDOWN:
                if event.key == pygame.K_r:
                    return True  # restart
                if event.key == pygame.K_q:
                    pygame.quit()
                    sys.exit()

def main():
    screen = pygame.display.set_mode((WIDTH, HEIGHT))
    pygame.display.set_caption("Snake Game")
    clock = pygame.time.Clock()

    score = 0
    snake = Snake()
    food = Food(snake.positions)
    direction = RIGHT
    game_over = False

    # Main game loop
    while True:
        # Event handling
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                pygame.quit()
                sys.exit()
            if event.type == pygame.KEYDOWN:
                if not game_over:
                    if event.key == pygame.K_UP:
                        direction = UP
                    elif event.key == pygame.K_DOWN:
                        direction = DOWN
                    elif event.key == pygame.K_LEFT:
                        direction = LEFT
                    elif event.key == pygame.K_RIGHT:
                        direction = RIGHT

        if not game_over:
            snake.change_direction(direction)
            snake.move()

            # Check food collision
            if snake.get_head() == food.position:
                snake.grow()
                score += 1
                food.randomize(snake.positions)

            # Check collision with walls or self
            if snake.check_collision():
                game_over = True

            # Draw everything
            screen.fill(BLACK)

            # Draw grid lines (subtle)
            for x in range(0, WIDTH, CELL_SIZE):
                pygame.draw.line(screen, (30, 30, 30), (x, 0), (x, HEIGHT))
            for y in range(0, HEIGHT, CELL_SIZE):
                pygame.draw.line(screen, (30, 30, 30), (0, y), (WIDTH, y))

            food.draw(screen)
            snake.draw(screen)

            # Draw score
            font = pygame.font.Font(None, 36)
            score_text = font.render(f"Score: {score}", True, WHITE)
            screen.blit(score_text, (10, 10))

            pygame.display.flip()

            # Speed up slightly as score increases
            current_fps = FPS + min(score, 15)  # cap at FPS+15
            clock.tick(current_fps)
        else:
            # Show game over screen and handle restart
            restart = show_game_over(screen, score)
            if restart:
                # Reset game
                score = 0
                snake = Snake()
                food = Food(snake.positions)
                direction = RIGHT
                game_over = False

if __name__ == "__main__":
    main()
