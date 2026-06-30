import pygame
import random
import sys

# Initialize Pygame
pygame.init()

# Constants
SCREEN_WIDTH = 600
SCREEN_HEIGHT = 600
GRID_SIZE = 20
CELL_SIZE = SCREEN_WIDTH // GRID_SIZE

# Colors (RGB)
BLACK = (0, 0, 0)
WHITE = (200, 200, 200)
GREEN = (0, 255, 0)
DARK_GREEN = (0, 200, 0)
RED = (255, 0, 0)
DARK_RED = (200, 0, 0)
YELLOW = (255, 255, 0)
GOLD = (218, 165, 32)
BLUE = (50, 50, 255)
DARK_BLUE = (0, 0, 200)
GRAY = (40, 40, 40)

# Directions
UP = (0, -1)
DOWN = (0, 1)
LEFT = (-1, 0)
RIGHT = (1, 0)

class Snake:
    def __init__(self):
        # Start in the middle of the screen
        start_x = GRID_SIZE // 2
        start_y = GRID_SIZE // 2
        self.body = [(start_x, start_y), (start_x - 1, start_y), (start_x - 2, start_y)]
        self.direction = RIGHT
        self.grow = False

    def move(self):
        head_x, head_y = self.body[0]
        dir_x, dir_y = self.direction
        new_head = (head_x + dir_x, head_y + dir_y)

        # Insert new head
        self.body.insert(0, new_head)

        if not self.grow:
            self.body.pop()
        else:
            self.grow = False

    def change_direction(self, new_dir):
        # Prevent reversing into itself
        opposite = (-self.direction[0], -self.direction[1])
        if new_dir != opposite:
            self.direction = new_dir

    def check_collision(self):
        head = self.body[0]
        # Wall collision
        if head[0] < 0 or head[0] >= GRID_SIZE or head[1] < 0 or head[1] >= GRID_SIZE:
            return True
        # Self collision
        if head in self.body[1:]:
            return True
        return False

    def draw(self, screen):
        for i, segment in enumerate(self.body):
            x, y = segment
            rect = pygame.Rect(x * CELL_SIZE, y * CELL_SIZE, CELL_SIZE, CELL_SIZE)
            if i == 0:
                # Head
                pygame.draw.rect(screen, DARK_BLUE, rect)
                # Eyes
                eye_color = WHITE
                eye_size = 3
                if self.direction == RIGHT:
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 14, y * CELL_SIZE + 6), eye_size)
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 14, y * CELL_SIZE + 14), eye_size)
                elif self.direction == LEFT:
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 6, y * CELL_SIZE + 6), eye_size)
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 6, y * CELL_SIZE + 14), eye_size)
                elif self.direction == UP:
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 6, y * CELL_SIZE + 6), eye_size)
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 14, y * CELL_SIZE + 6), eye_size)
                elif self.direction == DOWN:
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 6, y * CELL_SIZE + 14), eye_size)
                    pygame.draw.circle(screen, eye_color, (x * CELL_SIZE + 14, y * CELL_SIZE + 14), eye_size)
            else:
                pygame.draw.rect(screen, BLUE, rect)
                pygame.draw.rect(screen, BLACK, rect, 1)

    def get_head(self):
        return self.body[0]


class Food:
    def __init__(self, snake_body):
        self.position = self.random_position(snake_body)

    def random_position(self, snake_body):
        while True:
            x = random.randint(0, GRID_SIZE - 1)
            y = random.randint(0, GRID_SIZE - 1)
            if (x, y) not in snake_body:
                return (x, y)

    def draw(self, screen):
        x, y = self.position
        rect = pygame.Rect(x * CELL_SIZE, y * CELL_SIZE, CELL_SIZE, CELL_SIZE)
        pygame.draw.rect(screen, YELLOW, rect)
        pygame.draw.rect(screen, GOLD, rect, 2)
        # Inner highlight
        inner_rect = pygame.Rect(x * CELL_SIZE + 4, y * CELL_SIZE + 4, CELL_SIZE - 8, CELL_SIZE - 8)
        pygame.draw.rect(screen, (255, 255, 150), inner_rect)


def draw_grid(screen):
    for x in range(0, SCREEN_WIDTH, CELL_SIZE):
        pygame.draw.line(screen, GRAY, (x, 0), (x, SCREEN_HEIGHT))
    for y in range(0, SCREEN_HEIGHT, CELL_SIZE):
        pygame.draw.line(screen, GRAY, (0, y), (SCREEN_WIDTH, y))


def show_game_over(screen, score):
    font_large = pygame.font.Font(None, 72)
    font_small = pygame.font.Font(None, 36)

    overlay = pygame.Surface((SCREEN_WIDTH, SCREEN_HEIGHT))
    overlay.set_alpha(180)
    overlay.fill(BLACK)
    screen.blit(overlay, (0, 0))

    text1 = font_large.render("GAME OVER", True, RED)
    text1_rect = text1.get_rect(center=(SCREEN_WIDTH // 2, SCREEN_HEIGHT // 2 - 60))
    screen.blit(text1, text1_rect)

    text2 = font_small.render(f"Score: {score}", True, WHITE)
    text2_rect = text2.get_rect(center=(SCREEN_WIDTH // 2, SCREEN_HEIGHT // 2 + 20))
    screen.blit(text2, text2_rect)

    text3 = font_small.render("Press SPACE to restart or ESC to quit", True, WHITE)
    text3_rect = text3.get_rect(center=(SCREEN_WIDTH // 2, SCREEN_HEIGHT // 2 + 70))
    screen.blit(text3, text3_rect)

    pygame.display.flip()


def main():
    screen = pygame.display.set_mode((SCREEN_WIDTH, SCREEN_HEIGHT))
    pygame.display.set_caption("Snake Game")
    clock = pygame.time.Clock()

    snake = Snake()
    food = Food(snake.body)
    score = 0
    game_over = False

    # Font for score display
    font = pygame.font.Font(None, 36)

    # Game speed - increases as score increases
    base_speed = 10

    running = True
    while running:
        # Event handling
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                running = False
                pygame.quit()
                sys.exit()

            if event.type == pygame.KEYDOWN:
                if game_over:
                    if event.key == pygame.K_SPACE:
                        # Restart game
                        snake = Snake()
                        food = Food(snake.body)
                        score = 0
                        game_over = False
                    elif event.key == pygame.K_ESCAPE:
                        running = False
                        pygame.quit()
                        sys.exit()
                else:
                    if event.key == pygame.K_UP:
                        snake.change_direction(UP)
                    elif event.key == pygame.K_DOWN:
                        snake.change_direction(DOWN)
                    elif event.key == pygame.K_LEFT:
                        snake.change_direction(LEFT)
                    elif event.key == pygame.K_RIGHT:
                        snake.change_direction(RIGHT)

        if not game_over:
            # Move snake
            snake.move()

            # Check collision
            if snake.check_collision():
                game_over = True
                continue

            # Check food collision
            if snake.get_head() == food.position:
                snake.grow = True
                score += 1
                food = Food(snake.body)

            # Drawing
            screen.fill(BLACK)
            draw_grid(screen)
            snake.draw(screen)
            food.draw(screen)

            # Draw score
            score_text = font.render(f"Score: {score}", True, WHITE)
            screen.blit(score_text, (10, 10))

            pygame.display.flip()

            # Speed increases slightly with score
            speed = base_speed + min(score, 20) * 2
            clock.tick(speed)
        else:
            show_game_over(screen, score)

    pygame.quit()
    sys.exit()


if __name__ == "__main__":
    main()
